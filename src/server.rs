use anyhow::{Result, Error, bail};

use std::path::Path;
use std::fs::{self, DirEntry};
use std::time::Instant;
use std::sync::Arc;
use std::io::Write;
use std::vec;

use log::{set_logger, set_max_level, LevelFilter};
use log::{info, error, warn};

use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::utils;
use crate::logger;
use crate::args;
use crate::globals;
use crate::proto;

const LOGGER: logger::Logger = logger::Logger; 
const MD5_SIZE: usize = 32;


pub struct Server {
    args: args::Args,
    hashes: proto::HashDatabase,
}

impl Server {
    pub fn new(args: args::Args) -> Self {
        Server {
            args,
            hashes: Arc::new(Mutex::new(vec![])),
        }
    }

     pub async fn start(&mut self) {
        let level = match self.args.log_level.as_str() {
            "info" => LevelFilter::Info,
            "warn" => LevelFilter::Warn,
            "error" => LevelFilter::Error,
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            _ => LevelFilter::Info
        };

        utils::banner();

        set_logger(&LOGGER).map(|()| set_max_level(level)).unwrap();

        //
        // Verify that passed command line is somewhat valid
        //
        self.verify_cmdline();

        //
        // Initialize DB or merge
        // Note that merge initializes DB while merging
        //
        if self.args.merge {
            // Merge and initialize
            match self.merge_and_initialize() {
                Ok(_) => {},
                Err(e) => {
                    error!("Failed to merge and initialize \"duhashtsrv\".");
                    error!("{}", e);
                    std::process::exit(1);
                },
            }
        } else {
            // Initialize normally
            match self.initialize() {
                Ok(_) => {},
                Err(e) => {
                    error!("Failed to initialize \"duhashtsrv\".");
                    error!("{}", e);
                    std::process::exit(1);
                },
            }
        }

        //
        // If the test is defined, run test & exit
        //
        if self.args.test.len() != 0 {
            match self.test() {
                Ok(_) => {
                    std::process::exit(0);
                },
                Err(e) => {
                    error!("Failed to initialize \"duhashtsrv\".");
                    error!("{}", e);
                    std::process::exit(1);
                },
            }
        }

        match self.start_server().await {
            Ok(_) => {},
            Err(e) => {
                error!("Failed to start server.");
                error!("{}", e);
                std::process::exit(1);
            }
        }
    }

    fn verify_cmdline(&self) {
        let exists = Path::new(&self.args.hash_file).exists();

        if !exists {
            error!("Failed to start \"duhashtsrv\".");
            error!("Given hash file \"{}\" not found.", &self.args.hash_file);
            std::process::exit(1);
        }

        if self.args.test.len() != 0 && self.args.test.len() != MD5_SIZE {
            error!("Failed to start \"duhashtsrv\".");
            error!("Given test hash \"{}\" does not match 32 bytes.", &self.args.test);
            std::process::exit(1);
        }

        if !self.args.merge && Self::has_change_files() {
            warn!("Found change files in \"./{}/\".", globals::CHANGE_FILE_DIR);
            warn!("Contents will not be used.");
            warn!("Use \"--merge\" parameter to merge into database.");
            warn!("Note that \"--merge\" will update the on-disk hash file.");
        }
    } 

    //
    // Initialize the database, by reading the input file & pulling all hashes
    // in memory as u64 pairs.
    //
    fn initialize(&mut self) -> Result<()> {
        info!("Initializing \"duhashtsrv\" version {}.", globals::VERSION);

        let ingest_size: u64 = utils::get_size(self.args.hash_file.clone())?;
        let line_amount: usize = ingest_size as usize / (MD5_SIZE + 1);

        info!("Got ingest size: {} bytes.", ingest_size);
        info!("Calculated total: {} MD5 hashes.", line_amount);

        self.hashes = Arc::new(Mutex::new(Vec::with_capacity(line_amount)));
        let hashes: proto::HashDatabase = Arc::clone(&self.hashes);
        let mut hashes_lock = hashes.try_lock().unwrap();

        let now = Instant::now();

        if let Ok(lines) = utils::read_lines(self.args.hash_file.clone()) {
            for line in lines.map_while(Result::ok) {
                if line.len() != MD5_SIZE {
                    bail!("Got invalid hash, size > {} bytes.", MD5_SIZE);
                }

                let n1 = match u64::from_str_radix(&line[..16], 16) {
                    Ok(n) => n,
                    Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                };
                let n2 = match u64::from_str_radix(&line[16..], 16) {
                    Ok(n) => n,
                    Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                };

                hashes_lock.push((n1, n2));
            }
        }

        let elapsed = now.elapsed();

        info!("Finished ingesting hashes.");
        info!("Total time taken: {:.2?}.", elapsed);

        Ok(())
    }

    //
    // Merge and initialize.
    // Creates a backup of `hash_file`.
    // Parses all files in `globas::CHANGE_FILE_DIR` as (u64, u64).
    // Reads & parses `hash_file` in memory as (u64, u64).
    // Inserts all change file hashes.
    // Writes new database to `hash_file` & removes change files.
    //
    fn merge_and_initialize(&mut self) -> Result<()> {
        // Check for files
        // Initialize normally otherwise
        if !Self::has_change_files() {
            info!("Merge specified but no change files found.");
            match self.initialize() {
                Ok(_) => {},
                Err(e) => {
                    error!("Failed to initialize \"duhashtsrv\".");
                    error!("{}", e);
                    std::process::exit(1);
                },
            }
            return Ok(());
        }

        let now = Instant::now();

        //
        // Backup existing file
        //
        info!("Initializing \"duhashtsrv\" version {} with merge.", globals::VERSION);
        info!("Creating backup of the existing \"{}\" hash file.", self.args.hash_file);
        self.backup_hash_file()?;

        //
        // Read & parse all change files
        //
        info!("Parsing change files.");
        let mut new_hashes: Vec<(u64, u64)> = vec![]; 
        let paths = Self::get_change_file_paths()?;
        for path in &paths {

            info!("Parsing change file \"./{}\".", path.path().display());

            if let Ok(lines) = utils::read_lines(path.path()) {
                for line in lines.map_while(Result::ok) {
                    if line.len() != MD5_SIZE {
                        bail!("Got invalid hash, size > {} bytes.", MD5_SIZE);
                    }

                    let n1 = match u64::from_str_radix(&line[..16], 16) {
                        Ok(n) => n,
                        Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                    };
                    let n2 = match u64::from_str_radix(&line[16..], 16) {
                        Ok(n) => n,
                        Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                    };

                    new_hashes.push((n1, n2));
                }
            }
        }
        info!("Finished parsing change files.");
        info!("Got total of {} hashes.", new_hashes.len());


        //
        // Read the current hash file as database
        //
        info!("Parsing the existing hash database.");

        let ingest_size: u64 = utils::get_size(self.args.hash_file.clone())?;
        let line_amount: usize = ingest_size as usize / (MD5_SIZE + 1);

        info!("Got ingest size: {} bytes.", ingest_size);
        info!("Calculated total: {} MD5 hashes.", line_amount);

        self.hashes = Arc::new(Mutex::new(Vec::with_capacity(line_amount)));
        let hashes: proto::HashDatabase = Arc::clone(&self.hashes);
        let mut hashes_lock = hashes.try_lock().unwrap();

        if let Ok(lines) = utils::read_lines(self.args.hash_file.clone()) {
            for line in lines.map_while(Result::ok) {
                if line.len() != MD5_SIZE {
                    bail!("Got invalid hash, size > {} bytes.", MD5_SIZE);
                }

                let n1 = match u64::from_str_radix(&line[..16], 16) {
                    Ok(n) => n,
                    Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                };
                let n2 = match u64::from_str_radix(&line[16..], 16) {
                    Ok(n) => n,
                    Err(_) => bail!("Failed to parse \"{}\" as pair u64.", line),
                };

                hashes_lock.push((n1, n2));
            }
        }
        let hashes_count_old = hashes_lock.len();

        //
        // Insert the new hashes within the database
        //
        info!("Inserting new hashes within the database.");
        for new_hash in new_hashes {
            match hashes_lock.binary_search(&new_hash) {
                Ok(_) => {},
                Err(pos) => hashes_lock.insert(pos, new_hash),
            }
        }
        let hashes_count_new = hashes_lock.len();
        info!("Finished inserting all new hashes.");
        info!("Total new hashes added: {}.", hashes_count_new - hashes_count_old);

        //
        // Write changes to disk
        //
        if hashes_count_new - hashes_count_old > 0 {
            info!("Attempting to write changes to disk.");
            // Open with write (to overwrite)
            let mut hash_file = std::fs::OpenOptions::new()
                .write(true)
                .open(&self.args.hash_file)?;
            // Slow AF, is there a better way to do this??
            // tbh. we should't care as merging should not be performed often.
            for hash in hashes_lock.iter() {
                match hash_file.write_fmt(
                    format_args!("{:016X}{:016X}\n", hash.0, hash.1)) {
                    Ok(_) => {},
                    Err(error) => {
                        error!("Failed to write changes to hash file.");
                        bail!("{}", error);
                    }
                };
            }
        } else {
            info!("No new hashes added, nothing to write to disk.");
        }

        //
        // Remove change files after all is done
        //
        info!("Attempting to clean up change files.");
        for path in &paths {
            match fs::remove_file(path.path()) {
                Ok(_) => info!("Removed change file \"{}\".", path.path().display()),
                Err(error) => {
                    error!("Failed to remove change file \"{}\".", path.path().display());
                    // No point in bailing here
                    error!("{}", error);
                } 

            }
        }

        let elapsed = now.elapsed();

        info!("Finished merging hashes.");
        info!("Total time taken: {:.2?}.", elapsed);
        
        Ok(())
    }

    fn test(&self) -> Result<()> {
        info!("Running test with hash: \"{}\".", self.args.test);

        let now = Instant::now();

        if self.args.test.len() != MD5_SIZE {
            bail!("Got invalid hash, size > {} bytes.", MD5_SIZE);
        }

        let n1 = match u64::from_str_radix(&self.args.test[..16], 16) {
            Ok(n) => n,
            Err(_) => bail!("Failed to parse \"{}\" as pair u64.", self.args.test),
        };
        let n2 = match u64::from_str_radix(&self.args.test[16..], 16) {
            Ok(n) => n,
            Err(_) => bail!("Failed to parse \"{}\" as pair u64.", self.args.test),
        };

        let hashes: proto::HashDatabase = Arc::clone(&self.hashes);
        let hashes_lock = hashes.try_lock().unwrap();
        match hashes_lock.binary_search(&(n1, n2)) {
            Ok(pos) => info!("Test hash found at position {}.", pos + 1),
            Err(_) => info!("Test hash not found."),
        }
        
        let elapsed = now.elapsed();

        info!("Finished test search.");
        info!("Total time taken: {:.2?}.", elapsed);

        Ok(())
    }

    async fn start_server(&self) -> Result<()> {
        let address = format!("{}:{}", self.args.host, self.args.port);
        info!("Starting server on \"{}\".", address);

        let listener = TcpListener::bind(&address).await?;

        loop {
            let (mut socket, remote_address) = listener.accept().await?;

            info!("Received connection from {:?}", remote_address);

            let hashes: proto::HashDatabase = Arc::clone(&self.hashes);

            tokio::spawn(async move {
                proto::handle_client(&mut socket, &hashes).await;
            });
        }
    }

    fn has_change_files() -> bool {
        match std::fs::read_dir(globals::CHANGE_FILE_DIR) {
            Ok(files) => {
                if files.count() > 0 {
                    true
                } else {
                    false
                }
            },
            Err(_) => false,
        }
    }

    fn get_change_file_paths() -> Result<Vec<DirEntry>, Error> {
        let mut paths: Vec<DirEntry> = vec![];
        for path in fs::read_dir(globals::CHANGE_FILE_DIR)? {
            match path {
                Ok(path) => paths.push(path),
                Err(error) => bail!(error),
            }
        }
        Ok(paths)
    }

    fn backup_hash_file(&self) -> Result<()> {
        let backup_file_name = self.args.hash_file.clone() + ".bak";
        info!("Backing up hash file to \"{}\".", backup_file_name);
        match std::fs::copy(self.args.hash_file.clone(), backup_file_name) {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Failed to backup hash file.");
                bail!(error)
            }
        }
    }
}
