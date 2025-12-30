use std::fs::File;
use std::io::{self, BufRead};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};

use crate::globals;

pub fn banner() {
    println!("{}\n", globals::BANNER);
}

// The output is wrapped in a Result to allow matching on errors.
// Returns an Iterator to the Reader of the lines of the file.
pub fn read_lines<P>(filename: P) -> io::Result<io::Lines<io::BufReader<File>>>
where P: AsRef<Path>, {
    let file = File::open(filename)?;
    Ok(io::BufReader::new(file).lines())
}

pub fn get_size<P>(filename: P) -> io::Result<u64>
where P: AsRef<Path>, {
    let file = File::open(filename)?;
    Ok(file.metadata()?.size())
}

pub fn change_file_name() -> Result<String> {
    let epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(epoch) => epoch,
        Err(error) => bail!(error),
    };

    Ok(format!("{}.{}.txt", epoch.as_secs(), epoch.subsec_nanos()))
}
