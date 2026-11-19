use super::{replay_http::read_json, Result};
use nexusweave::{activation::Report, Run};
use serde_json::{json, value::RawValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tiny_http::{Method, Request};

const MAX_BODY: usize = 16 * 1024 * 1024;
type Rejection = (u16, String);

pub struct Store {
    directory: PathBuf,
}

pub struct Reader {
    directory: PathBuf,
    loaded: BTreeSet<PathBuf>,
}

impl Reader {
    pub fn new(root: &Path) -> Self {
        Self {
            directory: root.join("activation-reports"),
            loaded: BTreeSet::new(),
        }
    }

    pub fn refresh<'a>(
        &mut self,
        runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<Report>,
    ) -> Result<()> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let mut paths = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.retain(|path| {
            !self.loaded.contains(path)
                && path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.len() == 64
                            && name
                                .bytes()
                                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                    })
        });
        if paths.is_empty() {
            return Ok(());
        }
        paths.sort_unstable();
        let runs: BTreeMap<_, _> = runs.map(|run| (run.id.as_str(), run)).collect();
        for path in paths {
            let report =
                read_report(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if report_path(&self.directory, &report)? != path {
                return Err(format!(
                    "{}: report identity does not match its filename",
                    path.display()
                )
                .into());
            }
            // An evidence file previously supplied to `serve` may be absent this time.
            if let Some(&run) = runs
                .get(report.location().run_id.as_str())
                .filter(|run| has_target(run, &report))
            {
                report
                    .validate(std::slice::from_ref(run))
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                if let Some(index) = insertion_index(reports, &report)
                    .map_err(|(_, message)| format!("{}: {message}", path.display()))?
                {
                    reports.insert(index, report);
                }
                self.loaded.insert(path);
            }
        }
        Ok(())
    }
}

impl Store {
    // The caller holds the parent OTLP store's exclusive lock.
    pub fn open(root: &Path, runs: &[Run], reports: &mut Vec<Report>) -> Result<Self> {
        let mut reader = Reader::new(root);
        fs::create_dir_all(&reader.directory)?;
        sync_directory(root)?;
        reader.refresh(runs.iter(), reports)?;
        Ok(Self {
            directory: reader.directory,
        })
    }

    pub fn route<'a>(
        &self,
        request: &mut Request,
        runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<Report>,
    ) -> Option<(u16, Vec<u8>)> {
        if request.method() != &Method::Post || request.url() != "/api/activation-reports" {
            return None;
        }
        let encoded = request.headers().iter().any(|header| {
            header.field.equiv("Content-Encoding")
                && !header.value.as_str().eq_ignore_ascii_case("identity")
        });
        let result = if encoded {
            Err((
                415,
                "Send activation reports with identity Content-Encoding.".into(),
            ))
        } else {
            read_json::<Box<RawValue>>(request, MAX_BODY)
                .and_then(|raw| self.accept(raw.get(), runs, reports))
        };
        let (status, body) = match result {
            Ok(created) => (if created { 201 } else { 200 }, json!({})),
            Err((status, message)) => (status, json!({ "message": message })),
        };
        Some((status, body.to_string().into_bytes()))
    }

    fn accept<'a>(
        &self,
        raw: &str,
        runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<Report>,
    ) -> std::result::Result<bool, Rejection> {
        let report: Report = serde_json::from_str(raw)
            .map_err(|error| (400, format!("Invalid activation report: {error}")))?;
        let run = target_run(runs, &report).ok_or_else(|| {
            (
                409,
                "The target decision is unavailable. Send its trace before retrying the report."
                    .into(),
            )
        })?;
        report
            .validate(std::slice::from_ref(run))
            .map_err(|error| (400, error))?;
        let index = insertion_index(reports, &report)?;
        let path = report_path(&self.directory, &report).map_err(storage_error)?;
        if path.try_exists().map_err(storage_error)? {
            if index.is_some() && read_report(&path).map_err(storage_error)? != report {
                return Err(conflicting(&report));
            }
        } else {
            let mut pending =
                tempfile::NamedTempFile::new_in(&self.directory).map_err(storage_error)?;
            pending.write_all(raw.as_bytes()).map_err(storage_error)?;
            pending.as_file().sync_all().map_err(storage_error)?;
            pending
                .persist_noclobber(path)
                .map_err(|error| storage_error(error.error))?;
        }
        if let Some(index) = index {
            reports.insert(index, report);
        }
        // Retry directory sync even when a previous request published the file.
        sync_directory(&self.directory).map_err(storage_error)?;
        Ok(index.is_some())
    }
}

fn report_path(directory: &Path, report: &Report) -> io::Result<PathBuf> {
    let key = serde_json::to_vec(&(report.location(), report.id())).map_err(io::Error::other)?;
    Ok(directory.join(format!("{:x}.json", Sha256::digest(key))))
}

fn read_report(path: &Path) -> io::Result<Report> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(io::Error::other)
}

fn target_run<'a>(mut runs: impl Iterator<Item = &'a Run>, report: &Report) -> Option<&'a Run> {
    runs.find(|run| run.id == report.location().run_id && has_target(run, report))
}

fn has_target(run: &Run, report: &Report) -> bool {
    let target = report.location();
    run.calls
        .iter()
        .find(|call| call.id == target.call_id)
        .is_some_and(|call| {
            call.decisions
                .iter()
                .any(|decision| decision.id == target.decision_id)
        })
}

fn insertion_index(
    reports: &[Report],
    report: &Report,
) -> std::result::Result<Option<usize>, Rejection> {
    let key = (report.location(), report.id());
    match reports.binary_search_by(|existing| (existing.location(), existing.id()).cmp(&key)) {
        Ok(index) if &reports[index] != report => Err(conflicting(report)),
        Ok(_) => Ok(None),
        Err(index) => Ok(Some(index)),
    }
}

fn conflicting(report: &Report) -> Rejection {
    (
        409,
        format!("Conflicting activation report {}.", report.id()),
    )
}

fn storage_error(error: io::Error) -> Rejection {
    (503, format!("Could not persist activation report: {error}"))
}

fn sync_directory(path: &Path) -> io::Result<()> {
    if cfg!(unix) {
        File::open(path)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_restores_reports_without_duplicating_retries() {
        let directory = tempfile::tempdir().unwrap();
        let raw =
            include_str!("../examples/runtime-comparison/generation-branches-transformers.json")
                .trim();
        let runs: Vec<Run> =
            serde_json::from_str(include_str!("../examples/runtime-comparison/evidence.json"))
                .unwrap();
        let mut reports = Vec::new();
        let store = Store::open(directory.path(), &runs, &mut reports).unwrap();
        assert_eq!(
            store
                .accept(raw, [].into_iter(), &mut reports)
                .unwrap_err()
                .0,
            409
        );
        let mut edited: serde_json::Value = serde_json::from_str(raw).unwrap();
        edited["baseline_answer"] = json!("Different output");
        assert_eq!(
            store
                .accept(&edited.to_string(), runs.iter(), &mut reports)
                .unwrap_err()
                .0,
            400
        );
        assert_eq!(fs::read_dir(&store.directory).unwrap().count(), 0);

        assert!(store.accept(raw, runs.iter(), &mut reports).unwrap());
        assert!(!store.accept(raw, runs.iter(), &mut reports).unwrap());
        assert_eq!(reports.len(), 1);
        let path = report_path(&store.directory, &reports[0]).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), raw);

        let mut restored = Vec::new();
        let store = Store::open(directory.path(), &runs, &mut restored).unwrap();
        assert_eq!(restored, reports);
        assert!(!store.accept(raw, runs.iter(), &mut restored).unwrap());
        let changed = raw.replace("qwen-return-and-refund\",", "a-different-case\",");
        assert_eq!(
            store
                .accept(&changed, runs.iter(), &mut restored)
                .unwrap_err()
                .0,
            409
        );
        assert_eq!(fs::read_dir(&store.directory).unwrap().count(), 1);
    }
}
