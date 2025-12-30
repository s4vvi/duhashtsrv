use anyhow::{Result, bail};

use std::fs;
use std::io::Write;
use std::time::Instant;
use std::sync::Arc;
use std::fs::File;

use log::{info, error};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::globals;
use crate::utils;

pub type HashDatabase = Arc<Mutex<Vec<(u64, u64)>>>;

//
// Here is the application layer protocol
// Allows for maximum of 65535 hashes per query
// This is more than enough, recommend maximum of few thousand
//
// +---------+---------+-----------------+-------------------------------+
// | Version | Command |     Length      |      Argumetns / Hashes       |
// +---------+---------+-----------------+-------------------------------+
// | 1 byte  | 1 byte  | 2 bytes (u16be) | Length * (u64be, u64be) bytes |
// +---------+---------+-----------------+-------------------------------+
//
// See `From<u8>` implementations for both ProtoVersion & ProtoCommand
// for available versions & commands.
// 
// Responses:
// +--------+---------+
// | Status |  Data   |
// +--------+---------+
// | 1 byte | * bytes |
// +--------+---------+
//

const ERROR_INVALID_LENGTH: &str = "ERROR_INVALID_LENGTH";
const ERROR_INVALID_PROTO_VERSION: &str = "ERROR_INVALID_PROTO_VERSION";
const ERROR_INVALID_COMMAND: &str = "ERROR_INVALID_COMMAND";
const ERROR_READ_FAIL: &str = "ERROR_READ_FAIL";
const ERROR_CHANGE_DIR_CHECK_FAIL: &str = "ERROR_CHANGE_DIR_CHECK_FAIL";
const ERROR_CHANGE_FILE_CREATE_FAIL: &str = "ERROR_CHANGE_FILE_CREATE_FAIL";
const ERROR_CHANGE_FILE_WRITE_FAIL: &str = "ERROR_CHANGE_FILE_WRITE_FAIL";
const ERROR_CHANGE_FILE_REMOVE_FAIL: &str = "ERROR_CHANGE_FILE_REMOVE_FAIL";

enum ProtoVersion {
    V1,
    Unknown,
}

impl From<u8> for ProtoVersion {
    fn from(byte: u8) -> Self {
        match byte {
            b'1' => ProtoVersion::V1,
            _ => ProtoVersion::Unknown,
        }
    }
}

enum ProtoCommand {
    Query,
    Update,
    End,
    Unknown,
}

impl From<u8> for ProtoCommand {
    fn from(byte: u8) -> Self {
        match byte {
            b'q' => ProtoCommand::Query,
            b'u' => ProtoCommand::Update,
            b'e' => ProtoCommand::End,
            _ => ProtoCommand::Unknown,
        }
    }
}

enum ProtoResponseStatus {
    Success,
    Error,
}

impl Into<u8> for ProtoResponseStatus {
    fn into(self) -> u8 {
        match self {
            Self::Success => b's',
            Self::Error => b'e',
        }
    }
}

pub async fn handle_client(socket: &mut TcpStream, hashes: &HashDatabase) {
    match handle_connection(socket, &hashes).await {
        Ok(_) => {},
        Err(error) => {
            match socket.write_u8(ProtoResponseStatus::Error.into()).await {
                Ok(_) => {},
                Err(error) => {
                    error!("Failed to write error to client.");
                    error!("{}", error);
                }
            };
            match socket.write_all(error.to_string().as_bytes()).await {
                Ok(_) => {},
                Err(error) => {
                    error!("Failed to write error to client.");
                    error!("{}", error);
                }
            };
        }
    };
    match socket.shutdown().await {
        Ok(_) => {},
        Err(error) => {
            error!("Failed to close connection.");
            error!("{}", error);
        }
        
    };
}

pub async fn handle_connection(socket: &mut TcpStream, hashes: &HashDatabase) -> Result<()> {
    loop {
        //
        // Read the first byte as the protocol version 
        // 
        match ProtoVersion::from(socket.read_u8().await.unwrap_or_else(|_| 0)) {
            //
            // Handle version 1
            // Likely the only version there will ever be but still
            //
            ProtoVersion::V1 => {
                //
                // Read next byte as the command
                // Handle all cases
                //
                match ProtoCommand::from(socket.read_u8().await.unwrap_or_else(|_| 0)) {
                    ProtoCommand::Query => handle_v1_query(socket, &hashes).await?,
                    ProtoCommand::Update => handle_v1_update(socket, &hashes).await?,
                    ProtoCommand::End => break,
                    ProtoCommand::Unknown => {
                        error!("Received invalid protocol command.");
                        bail!(ERROR_INVALID_COMMAND);
                    },
                }
            }
            ProtoVersion::Unknown => {
                error!("Received invalid protocol version.");
                bail!(ERROR_INVALID_PROTO_VERSION);
            },
        };
    }
    Ok(())
}

async fn handle_v1_query(socket: &mut TcpStream, hashes: &HashDatabase) -> Result<()> {
    let hash_count = match socket.read_u16().await {
        Ok(n) => n,
        Err(_) => {
            error!("Failed to receive length.");
            bail!(ERROR_INVALID_LENGTH);
        }
    };

    info!("Received a query with {} hashes.", hash_count);

    let now = Instant::now();
    let mut results: Vec<u8> = vec![]; 

    let hashes_lock = hashes.lock().await;

    for _ in 0..hash_count {
        let n1 = match socket.read_u64().await {
            Ok(n) => n,
            Err(_) => bail!(ERROR_READ_FAIL),
        };
        let n2 = match socket.read_u64().await {
            Ok(n) => n,
            Err(_) => bail!(ERROR_READ_FAIL),
        };

        match hashes_lock.binary_search(&(n1, n2)) {
            Ok(_) => results.push(1),
            Err(_) => results.push(0),
        }
    }
    drop(hashes_lock);

    let elapsed = now.elapsed();

    info!("Total time taken: {:.2?}.", elapsed);

    socket.write_u8(ProtoResponseStatus::Success.into()).await?;
    socket.write_all(&results).await?;

    Ok(())
}

//
// Insert new hashes into in-memory database.
// Create a change file that contains a sorted list of added hashes.
//
// NOTE: This is an expensive operation as it has to shift the entire database 
// multiple times. For large hash sets it is recommended to use two servers. 
// One would be the cold storage of for example NSRL, and the other would be the
// hot storage of newly found hashes. The client would then have to query both.
//
async fn handle_v1_update(socket: &mut TcpStream, hashes: &HashDatabase) -> Result<()> {
    let hash_count = match socket.read_u16().await {
        Ok(n) => n,
        Err(_) => {
            error!("Failed to receive length.");
            bail!(ERROR_INVALID_LENGTH);
        }
    };

    info!("Received an update with {} hashes.", hash_count);

    let (mut change_file, change_file_path) = match create_change_file() {
        Ok(file) => file,
        Err(error) => bail!(error),
    };

    let now = Instant::now();

    let mut new_hashes: Vec<(u64, u64)> = vec![]; 
    let mut hashes_lock = hashes.lock().await;

    for _ in 0..hash_count {
        let n1 = match socket.read_u64().await {
            Ok(n) => n,
            Err(_) => bail!(ERROR_READ_FAIL),
        };
        let n2 = match socket.read_u64().await {
            Ok(n) => n,
            Err(_) => bail!(ERROR_READ_FAIL),
        };

        match hashes_lock.binary_search(&(n1, n2)) {
            Ok(_) => {}, // Element exists
            Err(pos) => {
                hashes_lock.insert(pos, (n1, n2));
                new_hashes.push((n1, n2));
            },
        }
    }
    drop(hashes_lock);

    info!("Inserted a total of {}/{} hashes.", new_hashes.len(), hash_count);
    info!("Hashes that already exist were not inserted.");

    if new_hashes.len() != 0 {
        new_hashes.sort();
        for hash in &new_hashes {
            match change_file.write_fmt(
                format_args!("{:016X}{:016X}\n", hash.0, hash.1)) {
                Ok(_) => {},
                Err(error) => {
                    error!("Failed to write change file.");
                    error!("{}", error);
                    bail!(ERROR_CHANGE_FILE_WRITE_FAIL);
                }
            };
        }

        info!("Wrote change to \"{}\".", change_file_path);
    } else {
        drop(change_file);
        match fs::remove_file(change_file_path) {
            Ok(_) => {},
            Err(error) => {
                error!("Failed to remove empty change file.");
                error!("{}", error);
                bail!(ERROR_CHANGE_FILE_REMOVE_FAIL);
            }
        };
        info!("No new hashes added, change file not created.");
    }

    let elapsed = now.elapsed();

    info!("Total time taken: {:.2?}.", elapsed);

    socket.write_u8(ProtoResponseStatus::Success.into()).await?;
    socket.write_u16(new_hashes.len() as u16).await?;

    Ok(())
}

fn create_change_file() -> Result<(File, String)> {
    //
    // Check for change file directory
    // Create if missing
    //
    match fs::exists(globals::CHANGE_FILE_DIR) {
        Ok(exists) => {
            if !exists {
                info!("Change file directory missing.");
                match fs::create_dir(globals::CHANGE_FILE_DIR) {
                    Ok(_) => info!("Created change file directory."),
                    Err(error) => {
                        error!("Failed to create change file directory.");
                        error!("{}", error);
                    }
                }
            }
        },
        Err(error) => {
            error!("Failed to check change file directory.");
            error!("{}", error);
            bail!(ERROR_CHANGE_DIR_CHECK_FAIL);
        }
    }
    //
    // Create change file
    //
    let change_file_name = match utils::change_file_name() {
        Ok(name) => name,
        Err(error) => {
            error!("Failed to generate change file name.");
            error!("{}", error);
            bail!(ERROR_CHANGE_FILE_CREATE_FAIL);
        }
    };
    let change_file_path = globals::CHANGE_FILE_DIR.to_owned() + 
        "/" + &change_file_name;

    match File::create(&change_file_path) {
        Ok(file) => Ok((file, change_file_path)),
        Err(error) => {
            error!("Failed to open change file.");
            error!("{}", error);
            bail!(ERROR_CHANGE_FILE_CREATE_FAIL);
        }
    }
}
