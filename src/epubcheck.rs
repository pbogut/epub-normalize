use crate::process::{self, Interrupted, TimedOut};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Finding {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub column: Option<i64>,
}
impl Finding {
    pub fn is_error(&self) -> bool {
        matches!(self.severity.to_lowercase().as_str(), "error" | "fatal")
    }
    fn diagnostic(&self) -> (String, String) {
        (
            self.code.clone(),
            self.message
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
    fn fingerprint(&self) -> (String, String, String) {
        let (code, message) = self.diagnostic();
        (code, message, self.path.clone().unwrap_or_default())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub available: bool,
    pub status: String,
    pub findings: Vec<Finding>,
    pub returncode: Option<i32>,
    pub message: Option<String>,
    pub occurrence_count: usize,
    pub error_count: usize,
}
impl CheckResult {
    pub fn unavailable(status: &str, message: &str) -> Self {
        Self {
            available: false,
            status: status.into(),
            findings: Vec::new(),
            returncode: None,
            message: Some(message.into()),
            occurrence_count: 0,
            error_count: 0,
        }
    }
    pub fn errors(&self) -> Vec<Finding> {
        self.findings
            .iter()
            .filter(|f| f.is_error())
            .cloned()
            .collect()
    }
}
#[derive(Debug)]
pub struct Comparison {
    pub available: bool,
    pub persisting: Vec<Finding>,
    pub resolved: Vec<Finding>,
    pub introduced: Vec<Finding>,
}
pub trait Checker {
    fn check(&self, epub: &Path) -> Result<CheckResult>;
}
#[derive(Default)]
pub struct Runner {
    pub executable: Option<String>,
    pub jar: Option<PathBuf>,
    pub java: String,
    pub timeout: f64,
}
impl Runner {
    pub fn from_env() -> Self {
        let timeout = env::var("EPUBCHECK_TIMEOUT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n > 0.0)
            .unwrap_or(120.0);
        Self {
            executable: env::var("EPUBCHECK_EXECUTABLE")
                .ok()
                .filter(|s| !s.is_empty()),
            jar: env::var_os("EPUBCHECK_JAR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            java: env::var("JAVA").unwrap_or("java".into()),
            timeout,
        }
    }
}
impl Checker for Runner {
    fn check(&self, epub: &Path) -> Result<CheckResult> {
        let mut command = if let Some(exe) = &self.executable {
            Command::new(exe)
        } else if let Some(jar) = &self.jar
            && jar.is_file()
        {
            let mut c = Command::new(&self.java);
            c.arg("-jar").arg(jar);
            c
        } else {
            return Ok(CheckResult::unavailable(
                "unavailable",
                "EPUBCheck executable/JAR is unavailable.",
            ));
        };
        command.arg(epub).args(["--json", "-"]);
        let timeout = if self.timeout.is_finite() && self.timeout > 0.0 {
            self.timeout
        } else {
            120.0
        };
        let output = match process::capture(
            &mut command,
            None,
            false,
            Duration::try_from_secs_f64(timeout).ok(),
        ) {
            Ok(output) => output,
            Err(error) if error.is::<Interrupted>() => return Err(error),
            Err(error) => {
                return Ok(CheckResult::unavailable(
                    if error.is::<TimedOut>() {
                        "timeout"
                    } else {
                        "unavailable"
                    },
                    &error.to_string(),
                ));
            }
        };
        let json = if !output.stdout.is_empty() {
            &output.stdout
        } else if !output.stderr.is_empty() {
            &output.stderr
        } else {
            "{}"
        };
        Ok(parse_report(json, output.status.code().unwrap_or(-1)))
    }
}
pub fn parse_report(json: &str, status: i32) -> CheckResult {
    let error = |message: &str| {
        let mut result = CheckResult::unavailable("error", message);
        result.returncode = Some(status);
        result
    };
    let Ok(payload) = serde_json::from_str::<Value>(json) else {
        return error("EPUBCheck returned invalid JSON.");
    };
    if !matches!(status, 0 | 1) {
        return error("EPUBCheck failed to complete validation.");
    }
    let Some(messages) = payload.get("messages").and_then(Value::as_array) else {
        return error("EPUBCheck returned an unexpected JSON report.");
    };
    let mut result = CheckResult {
        available: true,
        status: "ok".into(),
        findings: Vec::new(),
        returncode: Some(status),
        message: None,
        occurrence_count: 0,
        error_count: 0,
    };
    for item in messages.iter().filter(|i| i.is_object()) {
        let fallback = vec![item.clone()];
        let locations = item
            .get("locations")
            .and_then(Value::as_array)
            .unwrap_or(&fallback);
        let extra = item
            .get("additionalLocations")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as usize;
        let severity = item
            .get("severity")
            .or_else(|| item.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("error")
            .to_lowercase();
        let code = item
            .get("ID")
            .or_else(|| item.get("code"))
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let message = item
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        for location in locations
            .iter()
            .filter(|l| l.is_object())
            .map(Some)
            .chain(std::iter::repeat_n(None, extra))
        {
            let finding = Finding {
                severity: severity.clone(),
                code: code.clone(),
                message: message.clone(),
                path: location
                    .and_then(|l| {
                        l.get("path")
                            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                            .or_else(|| l.get("file"))
                    })
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                line: location.and_then(|l| l.get("line")).and_then(Value::as_i64),
                column: location
                    .and_then(|l| l.get("column"))
                    .and_then(Value::as_i64),
            };
            result.occurrence_count += 1;
            result.error_count += usize::from(finding.is_error());
            result.findings.push(finding);
        }
    }
    result
}
pub fn compare(input: &CheckResult, output: &CheckResult) -> Comparison {
    let mut before: BTreeMap<_, Vec<Finding>> = BTreeMap::new();
    let mut after: BTreeMap<_, Vec<Finding>> = BTreeMap::new();
    for f in input.errors() {
        before.entry(f.fingerprint()).or_default().push(f);
    }
    for f in output.errors() {
        after.entry(f.fingerprint()).or_default().push(f);
    }
    let mut result = Comparison {
        available: input.available && output.available,
        persisting: Vec::new(),
        resolved: Vec::new(),
        introduced: Vec::new(),
    };
    for (key, findings) in &before {
        let matches = after.remove(key).unwrap_or_default();
        let common = findings.len().min(matches.len());
        result.persisting.extend_from_slice(&matches[..common]);
        result.resolved.extend_from_slice(&findings[common..]);
        result.introduced.extend_from_slice(&matches[common..]);
    }
    result.introduced.extend(after.into_values().flatten());
    result.resolved.retain(|f| {
        if let Some(index) = result.introduced.iter().position(|c| {
            c.diagnostic() == f.diagnostic() && (f.path.is_none() || c.path.is_none())
        }) {
            result.persisting.push(result.introduced.remove(index));
            false
        } else {
            true
        }
    });
    result.introduced.retain(|f| {
        let known = input.errors().iter().any(|c| {
            c.diagnostic() == f.diagnostic()
                && if f.path.is_none() {
                    c.path.is_none()
                } else {
                    c.path.is_none() || c.path == f.path
                }
        });
        if known {
            result.persisting.push(f.clone());
            false
        } else {
            true
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn report(path: Option<&str>, extra: usize) -> CheckResult {
        parse_report(&json!({"messages": [{"ID": "RSC-005", "message": "Bad structure", "severity": "ERROR", "locations": [{"path": path}], "additionalLocations": extra}]}).to_string(), 1)
    }
    #[test]
    fn comparison_preserves_opaque_locations_but_rejects_new_resources() {
        let a = report(Some("a.xhtml"), 0);
        let b = report(Some("b.xhtml"), 0);
        assert_eq!(compare(&a, &b).introduced.len(), 1);
        let a = report(Some("a.xhtml"), 3);
        assert!(compare(&a, &b).introduced.is_empty());
        assert_eq!(compare(&a, &b).resolved.len(), 3);
        let b = report(Some("a.xhtml"), 9);
        assert!(compare(&a, &b).introduced.is_empty());
        assert_eq!(b.error_count, 10);
    }
    #[test]
    fn malformed_checker_reports_are_unavailable() {
        for s in ["", "[]", "{}", "{\"messages\":{}}"] {
            assert!(!parse_report(s, 0).available);
        }
        assert!(!parse_report("{\"messages\":[]}", 2).available);
    }
}
