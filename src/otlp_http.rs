use super::{header, Result};
use flate2::read::MultiGzDecoder;
use nexusweave::{otlp, Run};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use prost::Message;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tiny_http::{Request, Response};

const MAX_BODY: usize = 64 * 1024 * 1024;
const JSON: &str = "application/json";
const PROTOBUF: &str = "application/x-protobuf";

pub struct Journal {
    reader: Reader,
    _lock: File,
}

pub struct Reader {
    directory: PathBuf,
    next_batch: u64,
    archive: otlp::Archive,
}

impl Reader {
    pub fn open(directory: &Path) -> Result<Self> {
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
        Ok(Self {
            directory: directory.to_owned(),
            next_batch: 1,
            archive: otlp::Archive::default(),
        })
    }

    fn batch_path(&self, number: u64) -> PathBuf {
        self.directory.join(format!("{number:020}.json"))
    }

    fn restore(&mut self, number: u64, file: File, fixed_ids: &BTreeSet<String>) -> Result<()> {
        let path = self.batch_path(number);
        let document = serde_json::from_reader(io::BufReader::new(file))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let batch = self
            .archive
            .prepare(&document)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if let Some(run) = batch.runs().find(|run| fixed_ids.contains(&run.id)) {
            return Err(fixed_run_collision(run).into());
        }
        let next = next_number(number)?;
        self.archive.commit(batch);
        self.next_batch = next;
        Ok(())
    }

    pub fn refresh(&mut self, fixed_ids: &BTreeSet<String>) -> Result<()> {
        loop {
            let number = self.next_batch;
            let path = self.batch_path(number);
            match File::open(&path) {
                Ok(file) => self.restore(number, file, fixed_ids)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(format!("{}: {error}", path.display()).into()),
            }
        }
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.archive.runs()
    }
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self> {
        fs::create_dir_all(path)?;
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join(".lock"))?;
        lock.try_lock()
            .map_err(|error| format!("cannot lock OTLP store {}: {error}", path.display()))?;
        let mut files = Vec::new();
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            if let Some(number) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| stem.len() == 20)
                .and_then(|stem| stem.parse::<u64>().ok())
                .filter(|_| path.extension().is_some_and(|ext| ext == "json"))
            {
                files.push((number, path));
            }
        }
        files.sort_by_key(|(number, _)| *number);
        let mut reader = Reader::open(path)?;
        let fixed_ids = BTreeSet::new();
        for (number, file) in files {
            reader.restore(number, File::open(file)?, &fixed_ids)?;
        }
        Ok(Self {
            reader,
            _lock: lock,
        })
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.reader.runs()
    }

    pub fn receive(&mut self, request: Request, fixed_ids: &BTreeSet<String>) -> bool {
        let previous = self.reader.next_batch;
        receive(request, MAX_BODY, |document| {
            self.accept(document, fixed_ids)
        });
        self.reader.next_batch != previous
    }

    fn accept(
        &mut self,
        document: Value,
        fixed_ids: &BTreeSet<String>,
    ) -> std::result::Result<(), Rejection> {
        let batch = self
            .reader
            .archive
            .prepare(&document)
            .map_err(|error| (400, error))?;
        if let Some(run) = batch.runs().find(|run| fixed_ids.contains(&run.id)) {
            return Err((400, fixed_run_collision(run)));
        }
        if !batch.is_empty() {
            let next = next_number(self.reader.next_batch).map_err(storage_error)?;
            let path = self.reader.batch_path(self.reader.next_batch);
            let mut pending =
                tempfile::NamedTempFile::new_in(&self.reader.directory).map_err(storage_error)?;
            serde_json::to_writer(pending.as_file_mut(), &document)
                .map_err(|error| storage_error(io::Error::other(error)))?;
            pending.as_file().sync_all().map_err(storage_error)?;
            pending
                .persist_noclobber(path)
                .map_err(|error| storage_error(error.error))?;
            self.reader.archive.commit(batch);
            self.reader.next_batch = next;
        }
        // The batch may already have been written when the exporter retries an acknowledgment.
        #[cfg(unix)]
        File::open(&self.reader.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(storage_error)?;
        Ok(())
    }
}

pub fn receive(
    mut request: Request,
    limit: usize,
    accept: impl FnOnce(Value) -> std::result::Result<(), Rejection>,
) -> bool {
    let content_type = request_header(&request, "Content-Type")
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    let format = match content_type {
        Some(value) if value.eq_ignore_ascii_case(PROTOBUF) => Format::Protobuf,
        Some(value) if value.eq_ignore_ascii_case(JSON) => Format::Json,
        _ => {
            respond(
                request,
                Format::Json,
                Err((
                    415,
                    "send OTLP as application/json or application/x-protobuf".into(),
                )),
            );
            return false;
        }
    };
    let result = read_document(&mut request, format, limit).and_then(accept);
    let accepted = result.is_ok();
    respond(request, format, result);
    accepted
}

fn fixed_run_collision(run: &Run) -> String {
    format!(
        "live trace {} is already loaded from an evidence file",
        run.id
    )
}

#[derive(Clone, Copy)]
enum Format {
    Json,
    Protobuf,
}

type Rejection = (u16, String);

fn request_header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn read_document(
    request: &mut Request,
    format: Format,
    limit: usize,
) -> std::result::Result<Value, Rejection> {
    let gzip = match request_header(request, "Content-Encoding") {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("identity") => false,
        Some(value) if value.eq_ignore_ascii_case("gzip") => true,
        _ => return Err((415, "expected identity or gzip Content-Encoding".into())),
    };
    if request.body_length().is_some_and(|len| len > limit) {
        return Err((413, format!("OTLP request exceeds {limit} bytes")));
    }
    let input = bounded_read(request.as_reader(), limit)?;
    let input = if gzip {
        bounded_read(MultiGzDecoder::new(input.as_slice()), limit)?
    } else {
        input
    };
    // Empty protobuf messages decode successfully. The JSON empty envelope may omit resourceSpans.
    let document = match format {
        Format::Json if input.is_empty() => Ok(serde_json::json!({})),
        Format::Json => serde_json::from_slice(&input).map_err(|error| error.to_string()),
        Format::Protobuf => ExportTraceServiceRequest::decode(input.as_slice())
            .map_err(|error| error.to_string())
            .and_then(|export| serde_json::to_value(export).map_err(|error| error.to_string())),
    }
    .map_err(|error| (400, format!("invalid OTLP export: {error}")))?;
    if !document.is_object() {
        return Err((400, "expected a single OTLP export object".into()));
    }
    Ok(document)
}

fn bounded_read(reader: impl Read, limit: usize) -> std::result::Result<Vec<u8>, Rejection> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| (400, format!("could not read OTLP request: {error}")))?;
    if bytes.len() > limit {
        return Err((413, format!("OTLP request exceeds {limit} bytes")));
    }
    Ok(bytes)
}

// google.rpc.Status with only the optional message used by this receiver.
#[derive(Message)]
struct Status {
    #[prost(string, tag = "2")]
    message: String,
}

fn respond(request: Request, format: Format, result: std::result::Result<(), Rejection>) {
    let content_type = match format {
        Format::Json => JSON,
        Format::Protobuf => PROTOBUF,
    };
    let (status, bytes) = match (format, result) {
        (Format::Json, Ok(())) => (200, b"{}".to_vec()),
        (Format::Protobuf, Ok(())) => (200, ExportTraceServiceResponse::default().encode_to_vec()),
        (Format::Json, Err((status, message))) => (
            status,
            serde_json::json!({ "message": message })
                .to_string()
                .into_bytes(),
        ),
        (Format::Protobuf, Err((status, message))) => (status, Status { message }.encode_to_vec()),
    };
    let reply = Response::from_data(bytes)
        .with_status_code(status)
        .with_header(header("Content-Type", content_type));
    let _ = request.respond(reply);
}

fn storage_error(error: io::Error) -> Rejection {
    (503, format!("could not persist OTLP batch: {error}"))
}

fn next_number(number: u64) -> io::Result<u64> {
    number
        .checked_add(1)
        .ok_or_else(|| io::Error::other("OTLP store is out of batch numbers"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopens_acknowledged_batches_and_does_not_store_identical_retries() {
        let path = tempfile::tempdir().unwrap();
        let document: Value = serde_json::from_str(include_str!("../fixtures/otlp.json")).unwrap();
        let expected = otlp::import(&document).unwrap();
        let fixed: BTreeSet<_> = expected.iter().map(|run| run.id.clone()).collect();
        let mut journal = Journal::open(path.path()).unwrap();
        assert!(Journal::open(path.path()).is_err());
        assert_eq!(journal.accept(document.clone(), &fixed).unwrap_err().0, 400);
        assert!(!path.path().join("00000000000000000001.json").exists());
        journal.accept(document.clone(), &BTreeSet::new()).unwrap();
        journal.accept(document.clone(), &BTreeSet::new()).unwrap();
        assert_eq!(journal.reader.next_batch, 2);
        drop(journal);
        // A process may have been interrupted before publishing a temporary batch.
        fs::write(path.path().join(".incomplete"), b"{").unwrap();
        let mut reopened = Journal::open(path.path()).unwrap();
        assert_eq!(reopened.runs().cloned().collect::<Vec<_>>(), expected);
        reopened.accept(document, &BTreeSet::new()).unwrap();
        assert_eq!(reopened.reader.next_batch, 2);
    }
}
