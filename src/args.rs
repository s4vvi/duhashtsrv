use clap::{Parser, builder::PossibleValuesParser};
use crate::globals::LOG_LEVELS;

/// Duhastsrv usage...
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Host to run on.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Entry port. 
    #[arg(long, default_value_t = 1337)]
    pub port: u16,

    /// The log level to use.
    #[arg(
        long,
        default_value = "info",
        value_parser = PossibleValuesParser::new(LOG_LEVELS)
    )]
    pub log_level: String,

    /// Hash input file, sorted uppercase. 
    #[arg(long, required = true)]
    pub hash_file: String,

    /// Merge change files into hash_file.
    #[arg(long, action = clap::ArgAction::SetTrue, default_value_t = false)]
    pub merge: bool,

    /// Test the search with a hash. 
    #[arg(long, default_value = "")]
    pub test: String,
}
