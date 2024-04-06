use std::io;
use std::io::{Read, Write};
use std::{env, net::ToSocketAddrs};

use libdeflater::{Compressor, Decompressor};
use mio::Token;
use rand::prelude::*;

use rust_mc_bot::{start_bots, Address};
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
const UDS_PREFIX: &str = "unix://";

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        let name = args.first().unwrap();
        #[cfg(unix)]
        println!("usage: {} <ip:port or path> <count> [threads]", name);
        #[cfg(not(unix))]
        println!("usage: {} <ip:port> <count> [threads]", name);
        println!("example: {} localhost:25565 500", name);
        #[cfg(unix)]
        println!("example: {} unix:///path/to/socket 500", name);
        return Ok(());
    }

    let arg1 = args.get(1).unwrap();
    let arg2 = args.get(2).unwrap();
    let arg3 = args.get(3);

    let mut addrs = None;

    #[cfg(unix)]
    if arg1.starts_with(UDS_PREFIX) {
        addrs = Some(Address::UNIX(PathBuf::from(
            arg1[UDS_PREFIX.len()..].to_owned(),
        )));
    }

    if addrs.is_none() {
        let mut parts = arg1.split(':');
        let ip = parts.next().expect("no ip provided");
        let port = parts
            .next()
            .map(|port_string| port_string.parse().expect("invalid port"))
            .unwrap_or(25565u16);

        let server = (ip, port)
            .to_socket_addrs()
            .expect("Not a socket address")
            .next()
            .expect("No socket address found");

        addrs = Some(Address::TCP(server));
    }

    // Cant be none because it would have panicked earlier
    let addrs = addrs.unwrap();

    let count: u32 = arg2.parse().expect(&format!("{} is not a number", arg2));
    let mut cpus = 1.max(num_cpus::get()) as u32;

    if let Some(str) = arg3 {
        cpus = str.parse().expect(&format!("{} is not a number", arg2));
    }

    println!("cpus: {}", cpus);

    let count_per_thread = count / cpus;
    let mut extra = count % cpus;
    let mut names_used = 0;

    if count > 0 {
        let mut threads = Vec::new();
        for _ in 0..cpus {
            let mut count = count_per_thread;

            if extra > 0 {
                extra -= 1;
                count += 1;
            }

            let addrs = addrs.clone();
            threads.push(std::thread::spawn(move || {
                start_bots(count, addrs, names_used, cpus)
            }));

            names_used += count;
        }

        for thread in threads {
            let _ = thread.join();
        }
    }
    Ok(())
}
