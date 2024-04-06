use crate::packet_utils::Buf;
use crate::states::{login, play};
use libdeflater::{CompressionLvl, Compressor, Decompressor};
use mio::net::{TcpStream, UnixStream};
use mio::{event, Events, Interest, Poll, Registry, Token};
use rand::prelude::SliceRandom;
use rand::Rng;
use std::collections::HashMap;
use std::io;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod net;
mod packet_processors;
mod packet_utils;
mod states;

const SHOULD_MOVE: bool = true;

const PROTOCOL_VERSION: u32 = 763;

const MESSAGES: &[&str] = &["This is a chat message!", "Wow", "Server = on?"];

pub fn start_bots(count: u32, addrs: Address, name_offset: u32, cpus: u32) {
    if count == 0 {
        return;
    }
    let mut poll = Poll::new().expect("could not unwrap poll");
    //todo check used cap
    let mut events = Events::with_capacity((count * 5) as usize);
    let mut map = HashMap::new();

    println!("{:?}", addrs);

    fn start_bot(bot: &mut Bot, compression: &mut Compression) {
        bot.joined = true;

        // socket ops
        bot.stream.set_ops();

        //login sequence
        let buf = login::write_handshake_packet(PROTOCOL_VERSION, "".to_string(), 0, 2);
        bot.send_packet(buf, compression);

        let buf = login::write_login_start_packet(&bot.name);
        bot.send_packet(buf, compression);

        println!("bot \"{}\" joined", bot.name);
    }

    let bots_per_tick = (1.0 / cpus as f64).ceil() as u32;
    let mut bots_joined = 0;

    let mut packet_buf = Buf::with_length(2000);
    let mut uncompressed_buf = Buf::with_length(2000);

    let mut compression = Compression {
        compressor: Compressor::new(CompressionLvl::default()),
        decompressor: Decompressor::new(),
    };

    let dur = Duration::from_millis(50);

    let mut tick_counter = 0;
    let action_tick = 4;

    'main: loop {
        let ins = Instant::now();

        if bots_joined < count {
            let registry = poll.registry();
            for bot in bots_joined..(bots_per_tick + bots_joined).min(count) {
                let token = Token(bot as usize);
                let name = "Bot_".to_owned() + &(name_offset + bot).to_string();

                let mut bot = Bot {
                    token,
                    stream: addrs.connect(),
                    name,
                    id: bot,
                    entity_id: 0,
                    compression_threshold: 0,
                    state: 0,
                    kicked: false,
                    teleported: false,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    buffering_buf: Buf::with_length(200),
                    joined: false,
                };
                registry
                    .register(
                        &mut bot.stream,
                        bot.token,
                        Interest::READABLE | Interest::WRITABLE,
                    )
                    .expect("could not register");

                map.insert(token, bot);

                bots_joined += 1;
            }
        }

        poll.poll(&mut events, Some(dur)).expect("couldn't poll");
        for event in events.iter() {
            if let Some(bot) = map.get_mut(&event.token()) {
                if event.is_writable() && !bot.joined {
                    start_bot(bot, &mut compression);
                }
                if event.is_readable() && bot.joined {
                    net::process_packet(
                        bot,
                        &mut packet_buf,
                        &mut uncompressed_buf,
                        &mut compression,
                    );
                    if bot.kicked {
                        println!("{} disconnected", bot.name);
                        let token = bot.token;
                        map.remove(&token).expect("kicked bot doesn't exist");

                        if map.is_empty() {
                            break 'main;
                        }
                    }
                }
            }
        }

        let elapsed = ins.elapsed();
        if elapsed < dur {
            std::thread::sleep(dur - elapsed);
        }

        let mut to_remove = Vec::new();

        for bot in map.values_mut() {
            if SHOULD_MOVE && bot.teleported {
                bot.x += rand::random::<f64>() * 1.0 - 0.5;
                bot.z += rand::random::<f64>() * 1.0 - 0.5;
                bot.send_packet(play::write_current_pos(bot), &mut compression);

                if (tick_counter + bot.id) % action_tick == 0 {
                    match rand::thread_rng().gen_range(0..=4u8) {
                        0 => {
                            // Send chat
                            bot.send_packet(
                                play::write_chat_message(
                                    MESSAGES.choose(&mut rand::thread_rng()).unwrap(),
                                ),
                                &mut compression,
                            );
                        }
                        1 => {
                            // Punch animation
                            bot.send_packet(
                                play::write_animation(rand::random()),
                                &mut compression,
                            );
                        }
                        2 => {
                            // Sneak
                            bot.send_packet(
                                play::write_entity_action(
                                    bot.entity_id,
                                    if rand::random() { 1 } else { 0 },
                                    0,
                                ),
                                &mut compression,
                            );
                        }
                        3 => {
                            // Sprint
                            bot.send_packet(
                                play::write_entity_action(
                                    bot.entity_id,
                                    if rand::random() { 3 } else { 4 },
                                    0,
                                ),
                                &mut compression,
                            );
                        }
                        4 => {
                            // Held item
                            bot.send_packet(
                                play::write_held_slot(rand::thread_rng().gen_range(0..9)),
                                &mut compression,
                            );
                        }
                        _ => {}
                    }
                }
            }

            if bot.kicked {
                to_remove.push(bot.token);
            }
        }

        for bot in to_remove {
            let _ = map.remove(&bot);
        }

        tick_counter += 1;
    }
}

#[derive(Clone, Debug)]
pub enum Address {
    #[cfg(unix)]
    UNIX(PathBuf),
    TCP(SocketAddr),
}

impl Address {
    pub fn connect(&self) -> Stream {
        match self {
            #[cfg(unix)]
            Address::UNIX(path) => {
                Stream::UNIX(UnixStream::connect(path).expect("Could not connect to the server"))
            }
            Address::TCP(address) => Stream::TCP(
                TcpStream::connect(address.to_owned()).expect("Could not connect to the server"),
            ),
        }
    }
}

pub enum Stream {
    #[cfg(unix)]
    UNIX(UnixStream),
    TCP(TcpStream),
}

impl Stream {
    pub fn set_ops(&mut self) {
        // match self {
        //     Stream::TCP(s) => {
        //         s.set_nodelay(true).unwrap();
        //     }
        //     _ => {}
        // }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.read(buf),
            Stream::TCP(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.write(buf),
            Stream::TCP(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.flush(),
            Stream::TCP(s) => s.flush(),
        }
    }
}

impl event::Source for Stream {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.register(registry, token, interests),
            Stream::TCP(s) => s.register(registry, token, interests),
        }
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.reregister(registry, token, interests),
            Stream::TCP(s) => s.reregister(registry, token, interests),
        }
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::UNIX(s) => s.deregister(registry),
            Stream::TCP(s) => s.deregister(registry),
        }
    }
}

pub struct Compression {
    compressor: Compressor,
    decompressor: Decompressor,
}

pub struct Bot {
    pub token: Token,
    pub stream: Stream,
    pub name: String,
    pub id: u32,
    pub entity_id: u32,
    pub compression_threshold: i32,
    pub state: u8,
    pub kicked: bool,
    pub teleported: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub buffering_buf: Buf,
    pub joined: bool,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
