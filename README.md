# duhashtsrv

Hash search in-memory database server, intended for NSRL RDS type data sets. Rust rewrite of https://github.com/rjhansen/nsrlsvr. 

Added changes / features:
- smaller TCP protocol, reduces traffic thus increases speed;
- faster ingestion & search (Same methods, more optimal implementation);
- added runtime update queries.

Notes:
- updates are performed runtime and changes are written to `.change-files` (these act a sort of a WAL);
- updates from `.change-files` can later be merged in the actual hash file.

Build:
```bash
cargo build
```

Usage:
```
Duhastsrv usage...

Usage: duhashtsrv [OPTIONS] --hash-file <HASH_FILE>

Options:
      --host <HOST>            Host to run on [default: 127.0.0.1]
      --port <PORT>            Entry port [default: 1337]
      --log-level <LOG_LEVEL>  The log level to use [default: info] [possible values: info, warn, error, debug, trace]
      --hash-file <HASH_FILE>  Hash input file, sorted uppercase
      --merge                  Merge change files into hash_file
      --test <TEST>            Test the search with a hash [default: ]
  -h, --help                   Print help
  -V, --version                Print version
```

Examples:
```bash
# Run the server
duhashtsrv --host 127.0.0.1 --port 1337 --hash-file hashes.txt
# Run server w/ single test hash (dry-run)
duhashtsrv --host 127.0.0.1 --port 1337 --hash-file hashes.txt --test FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF 
# Run the server and merge any changes from `.change-files/` directory
duhashtsrv --host 127.0.0.1 --port 1337 --hash-file hashes.txt --merge
```

The tool is intended to be used w/ https://github.com/s4vvi/duhashtcli or https://github.com/s4vvi/duhashtgo. Examples:
```bash
# Query hashes, returns the misses
duhashtcli --host 127.0.0.1 --port 1337 query-file --hash-file hashes.txt
cat hashes.txt | duhashtcli --host 127.0.0.1 --port 1337 query-stdin
# Update hashes
duhashtcli --host 127.0.0.1 --port 1337 update-file --hash-file hashes.txt
cat hashes.txt | duhashtcli --host 127.0.0.1 --port 1337 update-stdin
```
