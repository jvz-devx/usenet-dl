//! NZB fixtures and test content generators

/// Minimal valid NZB for testing (single segment)
pub const MINIMAL_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">Test Download</meta>
  </head>
  <file poster="test@example.com" date="1234567890" subject="test.txt (1/1)">
    <groups>
      <group>alt.test</group>
    </groups>
    <segments>
      <segment bytes="100" number="1">test-msgid-12345@example.com</segment>
    </segments>
  </file>
</nzb>"#;

/// NZB with multiple segments for testing multi-part downloads
pub const MULTI_SEGMENT_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">Multi-Segment Test</meta>
  </head>
  <file poster="test@example.com" date="1234567890" subject="test.bin (1/3)">
    <groups>
      <group>alt.test</group>
    </groups>
    <segments>
      <segment bytes="1000" number="1">multi-part1@example.com</segment>
      <segment bytes="1000" number="2">multi-part2@example.com</segment>
      <segment bytes="500" number="3">multi-part3@example.com</segment>
    </segments>
  </file>
</nzb>"#;

/// NZB with password metadata
pub const PASSWORD_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">Password Protected</meta>
    <meta type="password">secret123</meta>
  </head>
  <file poster="test@example.com" date="1234567890" subject="protected.rar (1/1)">
    <groups>
      <group>alt.test</group>
    </groups>
    <segments>
      <segment bytes="5000" number="1">protected-rar@example.com</segment>
    </segments>
  </file>
</nzb>"#;

/// NZB with multiple files
pub const MULTI_FILE_NZB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">Multi-File Test</meta>
  </head>
  <file poster="test@example.com" date="1234567890" subject="file1.txt (1/1)">
    <groups>
      <group>alt.test</group>
    </groups>
    <segments>
      <segment bytes="100" number="1">file1-msgid@example.com</segment>
    </segments>
  </file>
  <file poster="test@example.com" date="1234567891" subject="file2.txt (1/1)">
    <groups>
      <group>alt.test</group>
    </groups>
    <segments>
      <segment bytes="200" number="1">file2-msgid@example.com</segment>
    </segments>
  </file>
</nzb>"#;

/// Generate an NZB from real message IDs
///
/// # Arguments
/// * `title` - Title for the NZB metadata
/// * `filename` - Subject filename
/// * `group` - Newsgroup name
/// * `segments` - List of (message_id, size_bytes) tuples
pub fn create_nzb_from_segments(
    title: &str,
    filename: &str,
    group: &str,
    segments: &[(String, u64)],
) -> String {
    let mut segments_xml = String::new();
    for (i, (message_id, size)) in segments.iter().enumerate() {
        segments_xml.push_str(&format!(
            "      <segment bytes=\"{}\" number=\"{}\">{}</segment>\n",
            size,
            i + 1,
            message_id
        ));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE nzb PUBLIC "-//newzBin//DTD NZB 1.1//EN" "http://www.newzbin.com/DTD/nzb/nzb-1.1.dtd">
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="title">{}</meta>
  </head>
  <file poster="test@example.com" date="{}" subject="{} (1/{})">
    <groups>
      <group>{}</group>
    </groups>
    <segments>
{}    </segments>
  </file>
</nzb>"#,
        title,
        chrono::Utc::now().timestamp(),
        filename,
        segments.len(),
        group,
        segments_xml
    )
}

/// Generate an NZB pointing to a single message ID
pub fn create_single_article_nzb(message_id: &str, size: u64, group: &str) -> String {
    create_nzb_from_segments(
        "Single Article Test",
        "test.bin",
        group,
        &[(message_id.to_string(), size)],
    )
}

/// Test content for posting to alt.test
pub const TEST_ARTICLE_CONTENT: &[u8] = b"This is test content for usenet-dl integration tests.\n\
    Line 2 of the test content.\n\
    Line 3 with some special chars: !@#$%^&*()\n";

/// Generate yEnc-encoded test content
///
/// Note: This is a simplified yEnc encoding for testing purposes.
/// Real yEnc encoding is more complex and handled by nntp-rs.
pub fn generate_yenc_content(data: &[u8], filename: &str) -> Vec<u8> {
    let mut result = Vec::new();

    // yEnc header
    let header = format!("=ybegin line=128 size={} name={}\r\n", data.len(), filename);
    result.extend_from_slice(header.as_bytes());

    // Simple encoding: escape special characters
    for &byte in data {
        let encoded = byte.wrapping_add(42);
        match encoded {
            0x00 | 0x0A | 0x0D | 0x3D => {
                result.push(b'=');
                result.push(encoded.wrapping_add(64));
            }
            _ => result.push(encoded),
        }
    }

    // yEnc footer
    let footer = format!("\r\n=yend size={}\r\n", data.len());
    result.extend_from_slice(footer.as_bytes());

    result
}
