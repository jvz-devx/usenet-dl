use crate::config::Config;
use crate::faults::FaultInjector;
use crate::protocol::{Command, ResponseGenerator};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

/// Fault injection NNTP server
pub struct Server {
    config: Config,
    shutdown: Arc<AtomicBool>,
    connection_count: Arc<AtomicUsize>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
            connection_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    pub async fn run(&self, port_override: Option<u16>) -> Result<(), ServerError> {
        let port = port_override.unwrap_or(self.config.server.port);
        let addr = format!("127.0.0.1:{}", port);

        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| ServerError::Bind(e.to_string()))?;

        info!(address = %addr, "Server listening");

        let semaphore = Arc::new(Semaphore::new(self.config.server.max_connections));

        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                info!("Shutdown requested");
                break;
            }

            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            let permit = semaphore.clone().try_acquire_owned();
                            if permit.is_err() {
                                warn!(peer = %peer_addr, "Connection rejected: max connections reached");
                                continue;
                            }

                            let config = self.config.clone();
                            let conn_count = self.connection_count.clone();
                            let conn_id = conn_count.fetch_add(1, Ordering::SeqCst);

                            info!(
                                connection_id = conn_id,
                                peer = %peer_addr,
                                "New connection"
                            );

                            tokio::spawn(async move {
                                let _permit = permit;
                                if let Err(e) = handle_connection(stream, config, conn_id).await {
                                    error!(
                                        connection_id = conn_id,
                                        error = %e,
                                        "Connection error"
                                    );
                                }
                                info!(connection_id = conn_id, "Connection closed");
                            });
                        }
                        Err(e) => {
                            error!(error = %e, "Accept error");
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    info!("Received Ctrl+C, shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    config: Config,
    conn_id: usize,
) -> Result<(), ConnectionError> {
    let injector = FaultInjector::new(config.faults.clone());
    let mut generator = ResponseGenerator::new(injector.clone());

    // Check for EOF on greeting fault
    if injector.should_eof_on_greeting() {
        info!(connection_id = conn_id, fault = "F1", "EOF on greeting");
        return Ok(());
    }

    // Check for connection hang
    let hang_ms = injector.get_connect_hang_ms();
    if hang_ms > 0 {
        info!(
            connection_id = conn_id,
            fault = "E1",
            duration_ms = hang_ms,
            "Hanging before greeting"
        );
        sleep(Duration::from_millis(hang_ms)).await;
    }

    // Send greeting
    let greeting = generator.greeting(&config.server.greeting);
    write_with_faults(&mut stream, &greeting, &injector).await?;

    // Check for close after greeting
    if injector.should_close_after_greeting() {
        info!(
            connection_id = conn_id,
            fault = "F1",
            "Closing after greeting"
        );
        return Ok(());
    }

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut command_count = 0usize;

    loop {
        line.clear();

        // Read command
        match reader.read_line(&mut line).await {
            Ok(0) => {
                debug!(connection_id = conn_id, "Client disconnected");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                debug!(connection_id = conn_id, error = %e, "Read error");
                break;
            }
        }

        let cmd = Command::parse(&line);
        debug!(connection_id = conn_id, command = ?cmd, "Received command");

        // Check for RST fault
        if injector.should_rst_connection() {
            info!(connection_id = conn_id, fault = "F3", "Simulating RST");
            // Just drop the connection abruptly
            break;
        }

        // Apply response delay
        let delay = injector.get_response_delay_ms();
        if delay > 0 {
            sleep(Duration::from_millis(delay)).await;
        }

        // Generate and send response
        let response = generator.process(cmd);
        let response_bytes = response.to_bytes();

        write_with_faults(&mut writer, &response_bytes, &injector).await?;

        if response.is_quit {
            break;
        }

        command_count += 1;

        // Check close_after_commands
        if config.faults.connection.close_after_commands > 0
            && command_count >= config.faults.connection.close_after_commands
        {
            info!(
                connection_id = conn_id,
                fault = "F8",
                commands = command_count,
                "Closing after N commands"
            );
            break;
        }
    }

    Ok(())
}

async fn write_with_faults<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    data: &[u8],
    injector: &FaultInjector,
) -> Result<(), ConnectionError> {
    // Check for freeze
    if let Some(freeze_ms) = injector.should_freeze() {
        // Write partial data, then freeze
        let partial_len = data.len() / 2;
        if partial_len > 0 {
            writer
                .write_all(&data[..partial_len])
                .await
                .map_err(|e| ConnectionError::Write(e.to_string()))?;
            writer
                .flush()
                .await
                .map_err(|e| ConnectionError::Write(e.to_string()))?;
        }

        sleep(Duration::from_millis(freeze_ms)).await;

        // Write rest
        writer
            .write_all(&data[partial_len..])
            .await
            .map_err(|e| ConnectionError::Write(e.to_string()))?;
    } else if let Some(byte_delay) = injector.get_byte_delay_ms() {
        // Slow drip mode
        for byte in data {
            writer
                .write_all(&[*byte])
                .await
                .map_err(|e| ConnectionError::Write(e.to_string()))?;
            writer
                .flush()
                .await
                .map_err(|e| ConnectionError::Write(e.to_string()))?;
            sleep(Duration::from_millis(byte_delay)).await;
        }
    } else {
        // Normal write
        writer
            .write_all(data)
            .await
            .map_err(|e| ConnectionError::Write(e.to_string()))?;
    }

    writer
        .flush()
        .await
        .map_err(|e| ConnectionError::Write(e.to_string()))?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("Failed to bind: {0}")]
    Bind(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("Write error: {0}")]
    Write(String),
}
