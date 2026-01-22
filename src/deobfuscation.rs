//! Obfuscated filename detection and handling
//!
//! Usenet releases often use obfuscated (random) filenames. This module provides
//! heuristics to detect such filenames and utilities to determine proper names.

use std::path::Path;

/// Check if a filename appears to be obfuscated (random/meaningless)
///
/// Uses multiple heuristics:
/// - High entropy (random alphanumeric)
/// - UUID-like patterns
/// - Pure hex strings
/// - No vowels (unlikely in real names)
///
/// # Examples
///
/// ```
/// use usenet_dl::deobfuscation::is_obfuscated;
///
/// assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0"));
/// assert!(is_obfuscated("550e8400-e29b-41d4-a716-446655440000.mkv"));
/// assert!(is_obfuscated("xkcd1234mnbvcxz.avi"));
/// assert!(!is_obfuscated("Movie.Name.2024.1080p.BluRay.x264.mkv"));
/// ```
pub fn is_obfuscated(filename: &str) -> bool {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    // Check for common obfuscation patterns
    let checks = [
        // Mostly random alphanumeric (high entropy)
        is_high_entropy(stem),
        // UUID-like patterns
        looks_like_uuid(stem),
        // Pure hex strings (longer than typical hashes in filenames)
        is_hex_string(stem) && stem.len() > 16,
        // Random with no vowels (unlikely in real names)
        has_no_vowels(stem) && stem.len() > 8,
    ];

    checks.iter().any(|&c| c)
}

/// Check if a string has high entropy (appears random)
///
/// Calculates a simple entropy measure by checking character distribution.
/// Returns true if the string appears to have random/uniform distribution.
///
/// This is intentionally conservative - it's better to miss some obfuscated files
/// than to false-positive on normal structured filenames.
fn is_high_entropy(s: &str) -> bool {
    if s.len() < 24 {
        // Need longer strings to be confident about randomness
        // Short strings can have apparent uniformity by chance
        return false;
    }

    // Count different character types
    let mut upper = 0;
    let mut lower = 0;
    let mut digit = 0;

    for c in s.chars() {
        match c {
            'A'..='Z' => upper += 1,
            'a'..='z' => lower += 1,
            '0'..='9' => digit += 1,
            _ => {} // Ignore separators like dots, hyphens
        }
    }

    let total = (upper + lower + digit) as f32;
    if total < 24.0 {
        return false;
    }

    let upper_ratio = upper as f32 / total;
    let lower_ratio = lower as f32 / total;
    let digit_ratio = digit as f32 / total;

    // High entropy: ALL three types must be present with very balanced distribution
    // This catches genuinely random strings like "aB3cD5eF7gH9iJ1kL2mN4oP6qR8sT0"
    // but not structured patterns like "EpisodeS01E01720pWEBDL"

    // Require all three types
    if upper == 0 || lower == 0 || digit == 0 {
        return false;
    }

    // Each type must be within tight range of 1/3
    // Real filenames rarely have this perfect balance
    let balanced_upper = upper_ratio >= 0.31 && upper_ratio <= 0.38;
    let balanced_lower = lower_ratio >= 0.31 && lower_ratio <= 0.38;
    let balanced_digit = digit_ratio >= 0.28 && digit_ratio <= 0.38;

    balanced_upper && balanced_lower && balanced_digit
}

/// Check if a string looks like a UUID
///
/// Matches patterns like: 550e8400-e29b-41d4-a716-446655440000
/// or without hyphens: 550e8400e29b41d4a716446655440000
fn looks_like_uuid(s: &str) -> bool {
    // UUID with hyphens: 8-4-4-4-12 hex digits
    if s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4 {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 5
            && parts[0].len() == 8
            && parts[1].len() == 4
            && parts[2].len() == 4
            && parts[3].len() == 4
            && parts[4].len() == 12
        {
            return parts.iter().all(|p| p.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    // UUID without hyphens: 32 hex digits
    if s.len() == 32 {
        return s.chars().all(|c| c.is_ascii_hexdigit());
    }

    false
}

/// Check if a string is entirely hexadecimal digits
fn is_hex_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Check if a string has no vowels
///
/// Real words and names almost always contain vowels.
/// Random strings may not have any vowels.
fn has_no_vowels(s: &str) -> bool {
    let vowels = ['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U'];
    !s.chars().any(|c| vowels.contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_high_entropy() {
        // High entropy (random-looking) - long with perfect balance
        assert!(is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4oP6qR8sT0uV2"));
        assert!(is_high_entropy("Xk4mP9wRt2Yz8QvN3Lb6Hj5Mk7Np1"));
        assert!(is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4oP6")); // Exactly 24 chars, perfectly balanced

        // Low entropy (structured)
        assert!(!is_high_entropy("MovieName2024"));
        assert!(!is_high_entropy("episode01"));
        assert!(!is_high_entropy("short")); // Too short
        assert!(!is_high_entropy("EpisodeS01E01720pWEBDL")); // Structured pattern, only 22 alphanumeric
        assert!(!is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4o")); // 23 chars - just under threshold
    }

    #[test]
    fn test_looks_like_uuid() {
        // Valid UUID patterns
        assert!(looks_like_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(looks_like_uuid("550e8400e29b41d4a716446655440000"));
        assert!(looks_like_uuid("A1B2C3D4-E5F6-7890-ABCD-EF1234567890"));

        // Not UUIDs
        assert!(!looks_like_uuid("not-a-uuid-at-all"));
        assert!(!looks_like_uuid("550e8400-e29b-41d4-a716")); // Too short
        assert!(!looks_like_uuid("550e8400-e29b-41d4-a716-446655440000-extra")); // Too long
    }

    #[test]
    fn test_is_hex_string() {
        assert!(is_hex_string("0123456789abcdef"));
        assert!(is_hex_string("ABCDEF123456"));
        assert!(is_hex_string("deadbeef"));

        assert!(!is_hex_string("not hex"));
        assert!(!is_hex_string("g123456")); // 'g' not hex
        assert!(!is_hex_string("")); // Empty
    }

    #[test]
    fn test_has_no_vowels() {
        assert!(has_no_vowels("xkcdmnbvcxz"));
        assert!(has_no_vowels("1234567890"));
        assert!(has_no_vowels("bcdfghjklmnpqrstvwxyz"));

        assert!(!has_no_vowels("hello"));
        assert!(!has_no_vowels("movie"));
        assert!(!has_no_vowels("A"));
    }

    #[test]
    fn test_is_obfuscated_uuid_patterns() {
        // UUID-like patterns should be detected as obfuscated
        assert!(is_obfuscated("550e8400-e29b-41d4-a716-446655440000.mkv"));
        assert!(is_obfuscated("550e8400e29b41d4a716446655440000.avi"));
    }

    #[test]
    fn test_is_obfuscated_hex_strings() {
        // Long hex strings should be detected
        assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0.mp4"));
        assert!(is_obfuscated("deadbeef1234567890abcdef.mkv"));

        // Short hex strings might be version/CRC codes (not obfuscated)
        assert!(!is_obfuscated("Movie[1a2b3c4d].mkv"));
    }

    #[test]
    fn test_is_obfuscated_no_vowels() {
        // Long strings with no vowels should be detected
        assert!(is_obfuscated("xkcd1234mnbvcxz.avi"));
        assert!(is_obfuscated("bcdfghjklmnp.mp4"));

        // Short strings are acceptable
        assert!(!is_obfuscated("cd1.mkv"));
    }

    #[test]
    fn test_is_obfuscated_high_entropy() {
        // Random alphanumeric should be detected
        assert!(is_obfuscated("aB3cD5eF7gH9iJ1kL2mN4oP6.mkv"));
        assert!(is_obfuscated("Xk4mP9wRt2Yz8QvN3Lb6.avi"));
    }

    #[test]
    fn test_is_obfuscated_normal_filenames() {
        // Normal filenames should NOT be detected as obfuscated
        assert!(!is_obfuscated("Movie.Name.2024.1080p.BluRay.x264.mkv"));
        assert!(!is_obfuscated("Episode.S01E01.720p.WEB-DL.mkv"));
        assert!(!is_obfuscated("Documentary.Title.2024.mp4"));
        assert!(!is_obfuscated("album_track01.mp3"));
        assert!(!is_obfuscated("my-vacation-video.avi"));
    }

    #[test]
    fn test_is_obfuscated_edge_cases() {
        // Empty/short filenames
        assert!(!is_obfuscated(""));
        assert!(!is_obfuscated("a.mkv"));
        assert!(!is_obfuscated("ab.mp4"));

        // Extension handling
        assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0")); // No extension
    }

    #[test]
    fn test_is_obfuscated_mixed_cases() {
        // Real-world examples from Usenet
        assert!(is_obfuscated("98234ksdfj2398sdkjf.avi")); // Random

        // Short hex strings are ambiguous (could be CRC codes)
        assert!(!is_obfuscated("7a4b9c2d.mkv")); // Could be CRC
        assert!(!is_obfuscated("[1a2b3c4d].mkv")); // CRC in brackets

        // Common false positives to avoid
        assert!(!is_obfuscated("x264.mkv")); // Codec name
        assert!(!is_obfuscated("h264.mp4")); // Codec name
        assert!(!is_obfuscated("BD1080p.mkv")); // Quality tag
    }
}
