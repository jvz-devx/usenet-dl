// Quick fault injection test - run with:
// cd /home/jens/Documents/source/usenet-dl && cargo test --test fault_injection_test -- --nocapture

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 1190;

fn connect() -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(format!("{}:{}", HOST, PORT))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}

fn read_line(reader: &mut BufReader<&TcpStream>) -> std::io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

fn send_command(stream: &mut TcpStream, cmd: &str) -> std::io::Result<()> {
    write!(stream, "{}\r\n", cmd)?;
    stream.flush()
}

#[test]
fn test_greeting() {
    let stream = connect().expect("Failed to connect");
    let mut reader = BufReader::new(&stream);

    let greeting = read_line(&mut reader).expect("Failed to read greeting");
    println!("Greeting: {:?}", greeting);
    println!("Greeting bytes: {:?}", greeting.as_bytes());

    // Check for various fault conditions
    if !greeting.starts_with("200") && !greeting.starts_with("201") {
        println!("WARNING: Unexpected greeting code");
    }

    // Check for control characters
    for (i, b) in greeting.as_bytes().iter().enumerate() {
        if *b < 32 && *b != b'\r' && *b != b'\n' {
            println!("WARNING: Control char 0x{:02x} at position {}", b, i);
        }
    }
}

#[test]
fn test_capabilities() {
    let mut stream = connect().expect("Failed to connect");
    let mut reader = BufReader::new(&stream);

    // Read greeting
    let _ = read_line(&mut reader).expect("Failed to read greeting");

    // Send CAPABILITIES
    send_command(&mut stream, "CAPABILITIES").expect("Failed to send");

    // Read response
    let status = read_line(&mut reader).expect("Failed to read status");
    println!("CAPABILITIES status: {:?}", status);

    // Read multiline response
    let mut lines = Vec::new();
    loop {
        let line = read_line(&mut reader).expect("Failed to read line");
        let trimmed = line.trim_end();
        if trimmed == "." {
            break;
        }
        lines.push(line.clone());

        // Safety limit
        if lines.len() > 100 {
            println!("WARNING: Too many lines, possible missing terminator");
            break;
        }
    }

    println!("Got {} capability lines", lines.len());
    for line in &lines {
        println!("  {:?}", line.trim_end());
    }
}

#[test]
fn test_article_fetch() {
    let mut stream = connect().expect("Failed to connect");
    let mut reader = BufReader::new(&stream);

    // Read greeting
    let _ = read_line(&mut reader).expect("Failed to read greeting");

    // Select group
    send_command(&mut stream, "GROUP misc.test").expect("Failed to send");
    let status = read_line(&mut reader).expect("Failed to read status");
    println!("GROUP status: {:?}", status);

    // Fetch article
    send_command(&mut stream, "ARTICLE 1").expect("Failed to send");
    let status = read_line(&mut reader).expect("Failed to read status");
    println!("ARTICLE status: {:?}", status);

    // Read multiline response
    let mut lines = Vec::new();
    let mut total_bytes = 0;
    loop {
        match read_line(&mut reader) {
            Ok(line) => {
                total_bytes += line.len();
                let trimmed = line.trim_end();
                if trimmed == "." {
                    break;
                }
                lines.push(line);

                if lines.len() > 1000 {
                    println!("WARNING: Too many lines");
                    break;
                }
            }
            Err(e) => {
                println!("ERROR reading article: {}", e);
                break;
            }
        }
    }

    println!("Article: {} lines, {} bytes", lines.len(), total_bytes);
}

#[test]
fn test_repeated_connections() {
    // Test connection resilience
    let mut successes = 0;
    let mut failures = 0;

    for i in 0..20 {
        match connect() {
            Ok(stream) => {
                let mut reader = BufReader::new(&stream);
                match read_line(&mut reader) {
                    Ok(greeting) => {
                        if greeting.contains("200") || greeting.contains("201") {
                            successes += 1;
                        } else {
                            println!("Iteration {}: Unexpected greeting: {:?}", i, greeting);
                            failures += 1;
                        }
                    }
                    Err(e) => {
                        println!("Iteration {}: Read error: {}", i, e);
                        failures += 1;
                    }
                }
            }
            Err(e) => {
                println!("Iteration {}: Connect error: {}", i, e);
                failures += 1;
            }
        }
    }

    println!("\nConnection test: {} successes, {} failures", successes, failures);
}

#[test]
fn test_xover() {
    let mut stream = connect().expect("Failed to connect");
    let mut reader = BufReader::new(&stream);

    // Read greeting
    let _ = read_line(&mut reader).expect("Failed to read greeting");

    // Select group
    send_command(&mut stream, "GROUP misc.test").expect("Failed to send");
    let _ = read_line(&mut reader).expect("Failed to read status");

    // XOVER
    send_command(&mut stream, "XOVER 1-10").expect("Failed to send");
    let status = read_line(&mut reader).expect("Failed to read status");
    println!("XOVER status: {:?}", status);

    // Read multiline
    let mut lines = Vec::new();
    loop {
        match read_line(&mut reader) {
            Ok(line) => {
                let trimmed = line.trim_end();
                if trimmed == "." {
                    break;
                }
                lines.push(line);
                if lines.len() > 100 {
                    break;
                }
            }
            Err(e) => {
                println!("XOVER read error: {}", e);
                break;
            }
        }
    }

    println!("XOVER returned {} lines", lines.len());
    for line in lines.iter().take(3) {
        println!("  {:?}", line.trim_end());
    }
}

fn main() {
    println!("Run with: cargo test --test fault_injection_test -- --nocapture");
}
