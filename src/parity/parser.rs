//! Parser for par2 command output

use super::traits::{RepairResult, VerifyResult};
use std::str;

/// Parse output from `par2 v` (verify) command
///
/// Parses the stdout/stderr from a par2 verify command to extract
/// information about file integrity, damaged blocks, and repair possibility.
///
/// # Arguments
///
/// * `stdout` - Standard output from the par2 command
/// * `stderr` - Standard error from the par2 command
/// * `success` - Whether the command exited successfully
///
/// # Returns
///
/// A `VerifyResult` containing verification details
pub fn parse_par2_verify_output(
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
) -> crate::Result<VerifyResult> {
    let output = str::from_utf8(stdout).unwrap_or_default();
    let error_output = str::from_utf8(stderr).unwrap_or_default();

    // Combine both outputs for parsing
    let combined = format!("{}\n{}", output, error_output);

    // Initialize result
    let mut damaged_blocks = 0;
    let mut recovery_blocks_available = 0;
    let mut damaged_files = Vec::new();
    let mut missing_files = Vec::new();

    // Parse output line by line
    for line in combined.lines() {
        let line_lower = line.to_lowercase();

        // Look for damaged/missing block counts
        if line_lower.contains("damaged") || line_lower.contains("missing") {
            // Try to extract numbers from patterns like "X blocks damaged" or "Missing X blocks"
            if let Some(count) = extract_number_before_blocks(&line_lower) {
                damaged_blocks = damaged_blocks.max(count);
            }
        }

        // Look for recovery block information
        if line_lower.contains("recovery") && line_lower.contains("block") {
            if let Some(count) = extract_number_before_blocks(&line_lower) {
                recovery_blocks_available = recovery_blocks_available.max(count);
            }
        }

        // Look for specific file mentions
        if line_lower.contains("damaged:") || line_lower.contains("corrupt:") {
            if let Some(filename) = line.split(':').nth(1) {
                let filename = filename.trim().to_string();
                if !filename.is_empty() && !damaged_files.contains(&filename) {
                    damaged_files.push(filename);
                }
            }
        }

        if line_lower.contains("missing:") {
            if let Some(filename) = line.split(':').nth(1) {
                let filename = filename.trim().to_string();
                if !filename.is_empty() && !missing_files.contains(&filename) {
                    missing_files.push(filename);
                }
            }
        }
    }

    // Determine completeness based on exit code and damaged blocks
    let is_complete = success && damaged_blocks == 0 && missing_files.is_empty();

    // Determine if repair is possible
    let repairable = damaged_blocks > 0 && recovery_blocks_available >= damaged_blocks;

    Ok(VerifyResult {
        is_complete,
        damaged_blocks,
        recovery_blocks_available,
        repairable,
        damaged_files,
        missing_files,
    })
}

/// Parse output from `par2 r` (repair) command
///
/// Parses the stdout/stderr from a par2 repair command to extract
/// information about repair success and which files were repaired.
///
/// # Arguments
///
/// * `stdout` - Standard output from the par2 command
/// * `stderr` - Standard error from the par2 command
/// * `success` - Whether the command exited successfully
///
/// # Returns
///
/// A `RepairResult` containing repair details
pub fn parse_par2_repair_output(
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
) -> crate::Result<RepairResult> {
    let output = str::from_utf8(stdout).unwrap_or_default();
    let error_output = str::from_utf8(stderr).unwrap_or_default();

    // Combine both outputs for parsing
    let combined = format!("{}\n{}", output, error_output);

    let mut repaired_files = Vec::new();
    let mut failed_files = Vec::new();
    let mut error = None;

    // Parse output line by line
    for line in combined.lines() {
        let line_lower = line.to_lowercase();

        // Look for repair success indicators
        if line_lower.contains("repaired") || line_lower.contains("restored") {
            if let Some(filename) = extract_filename_from_line(line) {
                if !repaired_files.contains(&filename) {
                    repaired_files.push(filename);
                }
            }
        }

        // Look for repair failures
        if line_lower.contains("failed") || line_lower.contains("could not repair") {
            if let Some(filename) = extract_filename_from_line(line) {
                if !failed_files.contains(&filename) {
                    failed_files.push(filename);
                }
            }
        }

        // Capture error messages
        if line_lower.contains("error") {
            if error.is_none() {
                error = Some(line.trim().to_string());
            }
        }
    }

    // If command failed but we don't have an error message, use stderr
    if !success && error.is_none() && !error_output.is_empty() {
        error = Some(error_output.trim().to_string());
    }

    Ok(RepairResult {
        success,
        repaired_files,
        failed_files,
        error,
    })
}

/// Extract a number that appears before the word "block" or "blocks" in a line
fn extract_number_before_blocks(line: &str) -> Option<u32> {
    // Look for patterns like "5 blocks" or "10 block"
    let words: Vec<&str> = line.split_whitespace().collect();
    for i in 0..words.len().saturating_sub(1) {
        if words[i + 1].starts_with("block") {
            if let Ok(num) = words[i].parse::<u32>() {
                return Some(num);
            }
        }
    }
    None
}

/// Extract filename from a line (look for quoted strings or after colons)
fn extract_filename_from_line(line: &str) -> Option<String> {
    // Try to find quoted filename first
    if let Some(start) = line.find('"') {
        if let Some(end) = line[start + 1..].find('"') {
            return Some(line[start + 1..start + 1 + end].to_string());
        }
    }

    // Try to find filename after colon
    if let Some(filename) = line.split(':').nth(1) {
        let filename = filename.trim().to_string();
        if !filename.is_empty() {
            return Some(filename);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_verify_success() {
        let stdout = b"All files are correct\nNo repair needed\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, true).unwrap();

        assert!(result.is_complete);
        assert_eq!(result.damaged_blocks, 0);
        assert!(result.damaged_files.is_empty());
        assert!(result.missing_files.is_empty());
    }

    #[test]
    fn test_parse_verify_with_damage() {
        let stdout = b"5 blocks damaged\n10 recovery blocks available\nDamaged: file1.bin\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, false).unwrap();

        assert!(!result.is_complete);
        assert_eq!(result.damaged_blocks, 5);
        assert_eq!(result.recovery_blocks_available, 10);
        assert!(result.repairable);
        assert_eq!(result.damaged_files, vec!["file1.bin"]);
    }

    #[test]
    fn test_parse_repair_success() {
        let stdout = b"Repaired: file1.bin\nRepaired: file2.bin\nRepair complete\n";
        let stderr = b"";
        let result = parse_par2_repair_output(stdout, stderr, true).unwrap();

        assert!(result.success);
        assert_eq!(result.repaired_files, vec!["file1.bin", "file2.bin"]);
        assert!(result.failed_files.is_empty());
    }

    #[test]
    fn test_parse_repair_failure() {
        let stdout = b"Failed to repair file1.bin\n";
        let stderr = b"Error: Not enough recovery blocks\n";
        let result = parse_par2_repair_output(stdout, stderr, false).unwrap();

        assert!(!result.success);
        assert_eq!(result.failed_files, vec!["file1.bin"]);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_extract_number_before_blocks() {
        assert_eq!(extract_number_before_blocks("5 blocks damaged"), Some(5));
        assert_eq!(extract_number_before_blocks("10 block available"), Some(10));
        assert_eq!(extract_number_before_blocks("damaged blocks"), None);
    }

    #[test]
    fn test_extract_filename_from_line() {
        assert_eq!(
            extract_filename_from_line("Repaired: \"file.bin\""),
            Some("file.bin".to_string())
        );
        assert_eq!(
            extract_filename_from_line("Damaged: file.bin"),
            Some("file.bin".to_string())
        );
    }
}
