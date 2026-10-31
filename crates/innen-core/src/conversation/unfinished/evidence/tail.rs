//! Reading the bounded transcript tail and the tool output it carries.

use serde_json::Value;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(super) fn bounded_tail_lines(path: &Path) -> Result<(VecDeque<String>, bool), std::io::Error> {
    const WINDOW: u64 = 1024 * 1024;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((size - start) as usize);
    file.take(size - start).read_to_end(&mut bytes)?;
    let mut lines = VecDeque::with_capacity(20);
    let mut first = true;
    for line in String::from_utf8_lossy(&bytes).lines() {
        if start > 0 && first {
            first = false;
            continue;
        }
        first = false;
        if lines.len() == 200 {
            lines.pop_front();
        }
        lines.push_back(line.to_owned());
    }
    Ok((lines, start > 0))
}

pub(super) fn extract_exit_code(output: &Value) -> Option<i32> {
    if let Some(obj) = output.as_object() {
        if let Some(code) = obj.get("exit_code").and_then(Value::as_i64) {
            return i32::try_from(code).ok();
        }
    }
    if let Some(s) = output.as_str() {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("Process exited with code ") {
                if let Ok(code) = rest.trim().parse::<i32>() {
                    return Some(code);
                }
            }
            if let Some(rest) = line.strip_prefix("The command exited with code ") {
                let trimmed = rest.trim().trim_end_matches('.');
                if let Ok(code) = trimmed.parse::<i32>() {
                    return Some(code);
                }
            }
        }
    }
    None
}
