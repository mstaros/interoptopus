//! Command-line entry point; the library documentation describes the request/response contract.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use interoptopus_rustdoc_gen::{Command, EngineError, ErrorCode, Response, run};

const USAGE: &str = "usage: interoptopus-rustdoc-gen <extract|analyze|generate> --request <file> --response <file>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match execute(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

/// Runs the command and writes the response file; returns whether the command succeeded.
fn execute(args: &[String]) -> Result<bool, EngineError> {
    let (command, request_path, response_path) = parse_args(args)?;
    let request = fs::read_to_string(&request_path)
        .map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot read {}: {e}", request_path.display())))?;
    let response = run(command, &request);
    write_response(&response_path, &response)?;
    Ok(response.is_success())
}

fn parse_args(args: &[String]) -> Result<(Command, PathBuf, PathBuf), EngineError> {
    let usage = || EngineError::new(ErrorCode::InvalidRequest, USAGE);
    let [command, first_flag, first_value, second_flag, second_value] = args else {
        return Err(usage());
    };
    let command = command.parse::<Command>()?;
    let mut request = None;
    let mut response = None;
    for (flag, value) in [(first_flag, first_value), (second_flag, second_value)] {
        let slot = match flag.as_str() {
            "--request" => &mut request,
            "--response" => &mut response,
            _ => return Err(usage()),
        };
        *slot = Some(PathBuf::from(value));
    }
    match (request, response) {
        (Some(request), Some(response)) => Ok((command, request, response)),
        _ => Err(usage()),
    }
}

fn write_response(path: &Path, response: &Response) -> Result<(), EngineError> {
    let json = serde_json::to_string_pretty(response).map_err(|e| EngineError::new(ErrorCode::Internal, e.to_string()))?;
    fs::write(path, json).map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot write {}: {e}", path.display())))
}