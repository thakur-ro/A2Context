//! Durable reports attached to decisions in the live trace store.

use super::{replay_http::read_json, Result};
use a2context::{activation, replay, Call, Run};
use serde::de::DeserializeOwned;
use serde_json::{json, value::RawValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use tiny_http::{Method, Request};

const MAX_BODY: usize = 16 * 1024 * 1024;
type Rejection = (u16, String);

pub trait Report: DeserializeOwned + PartialEq {
    const DIRECTORY: &'static str;
    const NAME: &'static str;

    fn location(&self) -> &replay::Location;
    fn id(&self) -> &str;
    fn validate_for(&self, run: &Run, call: &Call) -> std::result::Result<(), String>;
}

impl Report for activation::Report {
    const DIRECTORY: &'static str = "activation-reports";
    const NAME: &'static str = "activation report";

    fn location(&self) -> &replay::Location {
        self.location()
    }

    fn id(&self) -> &str {
        self.id()
    }

    fn validate_for(&self, run: &Run, _: &Call) -> std::result::Result<(), String> {
        self.validate(std::slice::from_ref(run))
    }
}

impl Report for replay::Replay {
    const DIRECTORY: &'static str = "replay-reports";
    const NAME: &'static str = "replay report";

    fn location(&self) -> &replay::Location {
        &self.location
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn validate_for(&self, run: &Run, call: &Call) -> std::result::Result<(), String> {
        if call.recorded_request.is_none() {
            return Err("Live replay reports require the captured inference request.".into());
        }
        replay::validate_report(self, std::slice::from_ref(run))
    }
}

pub struct Store<T> {
    directory: PathBuf,
    report: PhantomData<T>,
}

pub struct Reader<T> {
    directory: PathBuf,
    loaded: BTreeSet<PathBuf>,
    report: PhantomData<T>,
}

impl<T: Report> Reader<T> {
    pub fn new(root: &Path) -> Self {
        Self {
            directory: root.join(T::DIRECTORY),
            loaded: BTreeSet::new(),
            report: PhantomData,
        }
    }

    pub fn refresh<'a>(
        &mut self,
        runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<T>,
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
            let report: T =
                read_report(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if report_path(&self.directory, &report)? != path {
                return Err(format!(
                    "{}: report identity does not match its filename",
                    path.display()
                )
                .into());
            }
            // An evidence file previously supplied to `serve` may be absent this time.
            if let Some(&run) = runs.get(report.location().run_id.as_str()) {
                let Some(call) = target_call(run, report.location()) else {
                    continue;
                };
                report
                    .validate_for(run, call)
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

impl<T: Report> Store<T> {
    // The caller holds the parent OTLP store's exclusive lock.
    pub fn open(root: &Path, runs: &[Run], reports: &mut Vec<T>) -> Result<Self> {
        let mut reader = Reader::new(root);
        fs::create_dir_all(&reader.directory)?;
        sync_directory(root)?;
        reader.refresh(runs.iter(), reports)?;
        Ok(Self {
            directory: reader.directory,
            report: PhantomData,
        })
    }

    pub fn route<'a>(
        &self,
        request: &mut Request,
        runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<T>,
    ) -> Option<(u16, Vec<u8>)> {
        if request.method() != &Method::Post
            || request.url().strip_prefix("/api/") != Some(T::DIRECTORY)
        {
            return None;
        }
        let encoded = request.headers().iter().any(|header| {
            header.field.equiv("Content-Encoding")
                && !header.value.as_str().eq_ignore_ascii_case("identity")
        });
        let result = if encoded {
            Err((
                415,
                format!("Send {}s with identity Content-Encoding.", T::NAME),
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
        mut runs: impl Iterator<Item = &'a Run>,
        reports: &mut Vec<T>,
    ) -> std::result::Result<bool, Rejection> {
        let report: T = serde_json::from_str(raw)
            .map_err(|error| (400, format!("Invalid {}: {error}", T::NAME)))?;
        let location = report.location();
        let (run, call) = runs
            .find(|run| run.id == location.run_id)
            .and_then(|run| target_call(run, location).map(|call| (run, call)))
            .ok_or_else(|| {
                (
                    409,
                    "The target decision is unavailable. Send its trace before retrying the report."
                        .into(),
                )
            })?;
        report
            .validate_for(run, call)
            .map_err(|error| (400, error))?;
        let index = insertion_index(reports, &report)?;
        let path = report_path(&self.directory, &report).map_err(storage_error::<T>)?;
        if path.try_exists().map_err(storage_error::<T>)? {
            if index.is_some() && read_report::<T>(&path).map_err(storage_error::<T>)? != report {
                return Err(conflicting(&report));
            }
        } else {
            let mut pending =
                tempfile::NamedTempFile::new_in(&self.directory).map_err(storage_error::<T>)?;
            pending
                .write_all(raw.as_bytes())
                .map_err(storage_error::<T>)?;
            pending.as_file().sync_all().map_err(storage_error::<T>)?;
            pending
                .persist_noclobber(path)
                .map_err(|error| storage_error::<T>(error.error))?;
        }
        // Retry directory sync even when a previous request published the file.
        sync_directory(&self.directory).map_err(storage_error::<T>)?;
        if let Some(index) = index {
            reports.insert(index, report);
        }
        Ok(index.is_some())
    }
}

fn report_path(directory: &Path, report: &impl Report) -> io::Result<PathBuf> {
    let key = serde_json::to_vec(&(report.location(), report.id())).map_err(io::Error::other)?;
    Ok(directory.join(format!("{:x}.json", Sha256::digest(key))))
}

fn read_report<T: Report>(path: &Path) -> io::Result<T> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(io::Error::other)
}

fn target_call<'a>(run: &'a Run, target: &replay::Location) -> Option<&'a Call> {
    run.calls.iter().find(|call| {
        call.id == target.call_id
            && call
                .decisions
                .iter()
                .any(|decision| decision.id == target.decision_id)
    })
}

fn insertion_index<T: Report>(
    reports: &[T],
    report: &T,
) -> std::result::Result<Option<usize>, Rejection> {
    let key = (report.location(), report.id());
    match reports.binary_search_by(|existing| (existing.location(), existing.id()).cmp(&key)) {
        Ok(index) if &reports[index] != report => Err(conflicting(report)),
        Ok(_) => Ok(None),
        Err(index) => Ok(Some(index)),
    }
}

fn conflicting<T: Report>(report: &T) -> Rejection {
    (409, format!("Conflicting {} {}.", T::NAME, report.id()))
}

fn storage_error<T: Report>(error: io::Error) -> Rejection {
    (503, format!("Could not persist {}: {error}", T::NAME))
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
    fn live_replays_require_a_verified_request_and_restore_by_decision() {
        let directory = tempfile::tempdir().unwrap();
        let raw = include_str!("../fixtures/replay-report.json");
        let mut runs: Vec<Run> =
            serde_json::from_str(include_str!("../fixtures/replay-recording.json")).unwrap();
        let mut reports: Vec<replay::Replay> = Vec::new();
        let store = Store::open(directory.path(), &runs, &mut reports).unwrap();

        let recorded = runs[0].calls[0].recorded_request.take();
        let error = store.accept(raw, runs.iter(), &mut reports).unwrap_err();
        assert_eq!(error.0, 400);
        assert!(error.1.contains("require the captured inference request"));
        runs[0].calls[0].recorded_request = recorded;

        let mut changed: replay::Replay = serde_json::from_str(raw).unwrap();
        let baseline = std::mem::replace(&mut changed.baseline_request_sha256, "0".repeat(64));
        let error = store
            .accept(
                &serde_json::to_string(&changed).unwrap(),
                runs.iter(),
                &mut reports,
            )
            .unwrap_err();
        assert_eq!(error.0, 400);
        assert!(error.1.contains("baseline differs"));
        assert_eq!(fs::read_dir(&store.directory).unwrap().count(), 0);

        assert!(store.accept(raw, runs.iter(), &mut reports).unwrap());
        assert!(!store.accept(raw, runs.iter(), &mut reports).unwrap());
        changed.baseline_request_sha256 = baseline;
        changed.runner_id = "different-runner".into();
        let error = store
            .accept(
                &serde_json::to_string(&changed).unwrap(),
                runs.iter(),
                &mut reports,
            )
            .unwrap_err();
        assert_eq!(error.0, 409);

        let mut restored: Vec<replay::Replay> = Vec::new();
        let mut reader = Reader::new(directory.path());
        reader.refresh([].into_iter(), &mut restored).unwrap();
        assert!(restored.is_empty());
        reader.refresh(runs.iter(), &mut restored).unwrap();
        reader.refresh(runs.iter(), &mut restored).unwrap();
        assert_eq!(restored, reports);
    }
}
