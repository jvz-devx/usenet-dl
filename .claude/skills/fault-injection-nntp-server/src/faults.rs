use crate::config::FaultConfig;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand::Rng;
use std::io::Write;
use tracing::{debug, info};

/// Fault injector that applies configured faults to responses
#[derive(Clone)]
pub struct FaultInjector {
    config: FaultConfig,
}

impl FaultInjector {
    pub fn new(config: FaultConfig) -> Self {
        Self { config }
    }

    /// Check if we should apply a fault based on probability
    fn should_apply(&self, prob: f64) -> bool {
        if prob <= 0.0 {
            return false;
        }
        rand::thread_rng().gen::<f64>() < prob
    }

    /// Apply faults to a status line response
    pub fn apply_status_faults(&self, response: &str) -> Vec<u8> {
        let mut result = response.as_bytes().to_vec();

        // Check for malformed status
        if self.should_apply(self.config.response.malformed_status_prob) {
            result = self.malform_status(response);
        }

        // Check for encoding faults
        if self.should_apply(self.config.encoding.invalid_utf8_prob) {
            result = self.inject_invalid_utf8(&result);
        }

        if self.should_apply(self.config.encoding.nul_bytes_prob) {
            result = self.inject_nul_bytes(&result);
        }

        if self.should_apply(self.config.encoding.bom_prefix_prob) {
            result = self.add_bom_prefix(&result);
        }

        if self.should_apply(self.config.encoding.wrong_line_endings_prob) {
            result = self.wrong_line_endings(&result);
        }

        result
    }

    /// Apply faults to multiline response body
    pub fn apply_multiline_faults(&self, lines: &[String]) -> (Vec<u8>, bool) {
        let mut result = Vec::new();
        let mut include_terminator = true;

        for (i, line) in lines.iter().enumerate() {
            let mut line_bytes = line.as_bytes().to_vec();

            // Dot-stuffing for lines starting with .
            if line.starts_with('.') {
                line_bytes.insert(0, b'.');
            }

            // Apply line-level faults
            if self.should_apply(self.config.multiline.lone_cr_prob) {
                line_bytes.extend_from_slice(b"\r");
                info!(fault = "H1", "Injected lone CR terminator");
            } else if self.should_apply(self.config.multiline.mixed_line_endings_prob) {
                // Randomly use \n or \r\n
                if rand::thread_rng().gen::<bool>() {
                    line_bytes.extend_from_slice(b"\n");
                } else {
                    line_bytes.extend_from_slice(b"\r\n");
                }
                info!(fault = "H2", "Injected mixed line endings");
            } else {
                line_bytes.extend_from_slice(b"\r\n");
            }

            // Inject NUL bytes
            if self.should_apply(self.config.multiline.nul_in_body_prob) {
                let pos = rand::thread_rng().gen_range(0..line_bytes.len().max(1));
                line_bytes.insert(pos, 0);
                info!(fault = "H4", line = i, "Injected NUL byte in body");
            }

            // Check for truncation
            if self.should_apply(self.config.response.truncate_prob) {
                let truncate_at = rand::thread_rng().gen_range(0..line_bytes.len().max(1));
                result.extend_from_slice(&line_bytes[..truncate_at]);
                info!(fault = "B1", line = i, "Truncated response");
                return (result, false); // No terminator, connection should close
            }

            result.extend_from_slice(&line_bytes);
        }

        // Very long line injection
        if self.config.multiline.very_long_line_bytes > 0 {
            let long_line = vec![b'X'; self.config.multiline.very_long_line_bytes];
            result.extend_from_slice(&long_line);
            result.extend_from_slice(b"\r\n");
            info!(
                fault = "H3",
                bytes = self.config.multiline.very_long_line_bytes,
                "Injected very long line"
            );
        }

        // Missing terminator
        if self.should_apply(self.config.response.missing_terminator_prob) {
            include_terminator = false;
            info!(fault = "B1", "Omitting dot terminator");
        }

        // Double terminator
        if self.should_apply(self.config.multiline.double_terminator_prob) && include_terminator {
            result.extend_from_slice(b".\r\n.\r\n");
            info!(fault = "H7", "Injected double terminator");
            return (result, false); // Already added terminator
        }

        (result, include_terminator)
    }

    /// Malform a status line in various ways
    fn malform_status(&self, response: &str) -> Vec<u8> {
        let malformation = self
            .config
            .response
            .malformation_type
            .as_deref()
            .unwrap_or_else(|| {
                let types = [
                    "no_newline",
                    "nul_bytes",
                    "control_chars",
                    "tab_space",
                    "code_overflow",
                    "long_line",
                    "missing_code",
                    "letter_o",
                    "only_cr",
                    "double_space",
                ];
                types[rand::thread_rng().gen_range(0..types.len())]
            });

        let result = match malformation {
            "no_newline" => {
                // A1: Response with only \r (no \n)
                info!(fault = "A1", "Sending response with only CR");
                format!("{}\r", response.trim_end()).into_bytes()
            }
            "nul_bytes" => {
                // A2: NUL bytes in status
                info!(fault = "A2", "Injecting NUL bytes in status");
                let mut bytes = response.as_bytes().to_vec();
                if bytes.len() > 4 {
                    bytes.insert(4, 0);
                }
                bytes
            }
            "control_chars" => {
                // A3: Control characters
                info!(fault = "A3", "Injecting control characters");
                let parts: Vec<&str> = response.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    format!("{} \x01\x02\x03{}\r\n", parts[0], parts[1]).into_bytes()
                } else {
                    response.as_bytes().to_vec()
                }
            }
            "tab_space" => {
                // A4: Tab instead of space
                info!(fault = "A4", "Using tab instead of space");
                response.replacen(' ', "\t", 1).into_bytes()
            }
            "code_overflow" => {
                // A5: Code overflow (5 digits)
                info!(fault = "A5", "Sending 5-digit code");
                let trimmed = response.trim_end();
                if trimmed.len() >= 3 {
                    format!("99999{}\r\n", &trimmed[3..]).into_bytes()
                } else {
                    response.as_bytes().to_vec()
                }
            }
            "long_line" => {
                // A6: Very long status line (10KB)
                info!(fault = "A6", "Sending 10KB status line");
                let padding = "X".repeat(10240);
                format!("200 {}{}\r\n", padding, response).into_bytes()
            }
            "missing_code" => {
                // A9: Missing code
                info!(fault = "A9", "Sending response without code");
                b"OK ready\r\n".to_vec()
            }
            "letter_o" => {
                // I2: Letter O instead of zero
                info!(fault = "I2", "Using letter O instead of zero");
                response.replace('0', "O").into_bytes()
            }
            "only_cr" => {
                // A1 variant: Only CR
                info!(fault = "A1", "Sending only CR");
                format!("{}\r", response.trim_end()).into_bytes()
            }
            "double_space" => {
                // I4: Double space after code
                info!(fault = "I4", "Double space after code");
                response.replacen(' ', "  ", 1).into_bytes()
            }
            _ => response.as_bytes().to_vec(),
        };

        result
    }

    /// Inject invalid UTF-8 sequences
    fn inject_invalid_utf8(&self, data: &[u8]) -> Vec<u8> {
        let invalid_sequences: &[&[u8]] = &[
            &[0xC0, 0x80],       // C2: Overlong NUL
            &[0xFF, 0xFE],       // C3: Invalid sequence
            &[0xED, 0xA0, 0x80], // C1: Lone surrogate
            &[0xF8, 0x80, 0x80, 0x80, 0x80], // C7: 5-byte sequence
        ];

        let seq = invalid_sequences[rand::thread_rng().gen_range(0..invalid_sequences.len())];
        let mut result = data.to_vec();

        if result.len() > 4 {
            let pos = rand::thread_rng().gen_range(4..result.len() - 2);
            for (i, &byte) in seq.iter().enumerate() {
                if pos + i < result.len() {
                    result.insert(pos + i, byte);
                }
            }
        }

        info!(fault = "C1-C7", "Injected invalid UTF-8 sequence");
        result
    }

    /// Inject NUL bytes
    fn inject_nul_bytes(&self, data: &[u8]) -> Vec<u8> {
        let mut result = data.to_vec();
        if result.len() > 4 {
            let pos = rand::thread_rng().gen_range(4..result.len() - 2);
            result.insert(pos, 0);
            info!(fault = "A2/H4", position = pos, "Injected NUL byte");
        }
        result
    }

    /// Add BOM prefix
    fn add_bom_prefix(&self, data: &[u8]) -> Vec<u8> {
        let mut result = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        result.extend_from_slice(data);
        info!(fault = "C4", "Added BOM prefix");
        result
    }

    /// Use wrong line endings
    fn wrong_line_endings(&self, data: &[u8]) -> Vec<u8> {
        // Replace \r\n with just \n
        let mut result = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if i + 1 < data.len() && data[i] == b'\r' && data[i + 1] == b'\n' {
                result.push(b'\n');
                i += 2;
            } else {
                result.push(data[i]);
                i += 1;
            }
        }
        info!(fault = "H2", "Changed CRLF to LF only");
        result
    }

    /// Create a fake GZIP marker with plaintext
    pub fn apply_compression_faults(&self, data: &[u8], use_compression: bool) -> Vec<u8> {
        if self.should_apply(self.config.compression.fake_marker_prob) {
            // G3: Send marker but plaintext
            info!(fault = "G3", "Sending COMPRESS marker with plaintext");
            let mut result = b"[COMPRESS=GZIP]\r\n".to_vec();
            result.extend_from_slice(data);
            return result;
        }

        if self.should_apply(self.config.compression.corrupt_gzip_prob) && use_compression {
            // G2: Corrupt GZIP data
            info!(fault = "G2", "Corrupting GZIP data");
            let compressed = self.gzip_compress(data);
            let mut corrupted = compressed;
            if corrupted.len() > 10 {
                // Corrupt the middle
                let pos = corrupted.len() / 2;
                corrupted[pos] ^= 0xFF;
            }
            return corrupted;
        }

        if self.should_apply(self.config.compression.truncate_compressed_prob) && use_compression {
            // G5: Truncate compressed stream
            info!(fault = "G5", "Truncating compressed stream");
            let compressed = self.gzip_compress(data);
            let truncate_at = compressed.len() / 2;
            return compressed[..truncate_at].to_vec();
        }

        if self.should_apply(self.config.compression.decompression_bomb_prob) {
            // G6: Decompression bomb
            info!(
                fault = "G6",
                expanded_size = self.config.compression.bomb_expanded_size,
                "Sending decompression bomb"
            );
            let bomb_data = vec![0u8; self.config.compression.bomb_expanded_size];
            return self.gzip_compress(&bomb_data);
        }

        if self.should_apply(self.config.compression.missing_marker_prob) && use_compression {
            // G4: Compressed without marker
            info!(fault = "G4", "Sending compressed data without marker");
            return self.gzip_compress(data);
        }

        // No compression faults
        if use_compression {
            let mut result = b"[COMPRESS=GZIP]\r\n".to_vec();
            result.extend_from_slice(&self.gzip_compress(data));
            result
        } else {
            data.to_vec()
        }
    }

    fn gzip_compress(&self, data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// Get timing delay for slow drip
    pub fn get_byte_delay_ms(&self) -> Option<u64> {
        if self.config.timing.slow_drip_bytes_per_sec > 0 {
            Some(1000 / self.config.timing.slow_drip_bytes_per_sec as u64)
        } else {
            None
        }
    }

    /// Check if we should freeze mid-response
    pub fn should_freeze(&self) -> Option<u64> {
        if self.should_apply(self.config.timing.freeze_mid_response_prob) {
            info!(
                fault = "E3",
                duration_ms = self.config.timing.freeze_duration_ms,
                "Freezing mid-response"
            );
            Some(self.config.timing.freeze_duration_ms)
        } else {
            None
        }
    }

    /// Get response delay
    pub fn get_response_delay_ms(&self) -> u64 {
        self.config.timing.response_delay_ms
    }

    /// Check if should close after greeting
    pub fn should_close_after_greeting(&self) -> bool {
        self.config.connection.close_after_greeting
    }

    /// Get connection hang duration
    pub fn get_connect_hang_ms(&self) -> u64 {
        self.config.connection.hang_on_connect_ms
    }

    /// Check if should EOF on greeting
    pub fn should_eof_on_greeting(&self) -> bool {
        self.should_apply(self.config.connection.eof_on_greeting_prob)
    }

    /// Check if should RST connection
    pub fn should_rst_connection(&self) -> bool {
        self.should_apply(self.config.connection.rst_mid_connection_prob)
    }

    /// Apply invalid response code fault
    pub fn apply_invalid_code(&self, code: u16) -> String {
        if self.should_apply(self.config.response.invalid_code_prob) {
            let invalid_codes = [0, 1, 99, 1000, 9999];
            let new_code = invalid_codes[rand::thread_rng().gen_range(0..invalid_codes.len())];
            info!(
                fault = "D4/D5",
                original = code,
                new = new_code,
                "Replacing with invalid code"
            );
            new_code.to_string()
        } else {
            code.to_string()
        }
    }

    /// Apply article-specific faults
    pub fn apply_article_faults(&self, message_id: &str, headers: &mut Vec<String>, body: &mut Vec<String>) {
        // J1: Wrong message-id
        if self.should_apply(self.config.article.wrong_message_id_prob) {
            for header in headers.iter_mut() {
                if header.to_lowercase().starts_with("message-id:") {
                    *header = format!("Message-ID: <wrong-{}>", rand::thread_rng().gen::<u32>());
                    info!(
                        fault = "J1",
                        requested = message_id,
                        "Returning wrong message-id"
                    );
                    break;
                }
            }
        }

        // J3: Duplicate headers
        if self.should_apply(self.config.article.duplicate_headers_prob) {
            if let Some(subject) = headers.iter().find(|h| h.to_lowercase().starts_with("subject:")).cloned() {
                headers.push(subject);
                info!(fault = "J3", "Duplicated Subject header");
            }
        }

        // J2: Missing headers
        if self.should_apply(self.config.article.missing_headers_prob) {
            headers.clear();
            info!(fault = "J2", "Removed all headers");
        }

        // J6: yEnc corruption
        if self.should_apply(self.config.article.yenc_corruption_prob) {
            for line in body.iter_mut() {
                if line.starts_with("=ybegin") || line.starts_with("=ypart") || line.starts_with("=yend") {
                    // Corrupt the yEnc header
                    *line = line.replace("size=", "size=999999");
                    info!(fault = "J6", "Corrupted yEnc header");
                    break;
                }
            }
        }

        // J8: CRC mismatch
        if self.should_apply(self.config.article.crc_mismatch_prob) {
            for line in body.iter_mut() {
                if line.starts_with("=yend") && line.contains("crc32=") {
                    *line = line.replace("crc32=", "crc32=DEADBEEF");
                    info!(fault = "J8", "Corrupted CRC32");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_injector_default() {
        let config = FaultConfig::default();
        let injector = FaultInjector::new(config);

        // With default config (all probabilities 0), no faults should be applied
        let response = "200 OK\r\n";
        let result = injector.apply_status_faults(response);
        assert_eq!(result, response.as_bytes());
    }

    #[test]
    fn test_bom_injection() {
        let mut config = FaultConfig::default();
        config.encoding.bom_prefix_prob = 1.0; // Always inject

        let injector = FaultInjector::new(config);
        let response = "200 OK\r\n";
        let result = injector.apply_status_faults(response);

        assert!(result.starts_with(&[0xEF, 0xBB, 0xBF]));
    }
}
