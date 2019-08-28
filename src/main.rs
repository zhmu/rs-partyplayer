use std::io;
use std::io::{ BufRead, BufReader };
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process;
use std::process::{ Child, Command };
use std::{thread, time};

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
//use std::num::ParseIntError;

use ini::Ini;
//use ini::ini::Error;

extern crate tiny_http;
//use ascii::AsciiString;

#[derive(Debug)]
enum PlaylistError {
    Io(io::Error)
}

#[derive(Debug)]
enum StateError {
    Ini(ini::ini::Error),
    ParseInt(std::num::ParseIntError)
}

struct State {
    seed: u64,
    index: usize
}

impl State {
    fn new(path: &str) -> Result<State, StateError> {
        let seed:u64;
        let mut index:usize = 0;

        if Path::new(path).exists() {
            // INI file found; we'd better be able to parse it
            let conf = Ini::load_from_file(path).map_err(StateError::Ini)?;
            let section = conf.section(Some("general".to_owned())).unwrap();
            seed = section.get("seed").unwrap().parse::<u64>().map_err(StateError::ParseInt)?;
            index = section.get("index").unwrap().parse::<usize>().map_err(StateError::ParseInt)?;
        } else {
            // Generate a new random seed
            let mut rng = rand::thread_rng();
            seed = rng.gen::<u64>();
        }

        Ok(State { seed, index } )
    }

    fn write(&self, path: &str) -> Result<(), std::io::Error> {
        let mut conf = Ini::new();
        conf.with_section(Some("general".to_owned()))
            .set("seed", self.seed.to_string())
            .set("index", self.index.to_string());
        conf.write_to_file(path)
    }
}

struct Playlist {
    files: Vec<PathBuf>,
    index: usize
}

impl Playlist {
    fn new(path: &str, state: &State) -> Result<Playlist, PlaylistError> {
        let f = File::open(path).map_err(PlaylistError::Io)?;
        let f = BufReader::new(f);

        let mut files: Vec<PathBuf> = Vec::new();
        for l in f.lines() {
            let l = l.map_err(PlaylistError::Io)?;
            files.push(PathBuf::from(l));
        }

        let mut rng = StdRng::seed_from_u64(state.seed);
        files.shuffle(&mut rng);

        Ok(Playlist { files, index: state.index })
    }

    fn next(&self, state: &mut State) -> &PathBuf {
        let n = state.index;
        state.index += 1;
        &self.files[n]
    }

    fn print(&self, index: usize) {
        for (i, v) in self.files.iter().enumerate() {
            let marker = match index {
                _ if index == i => ">>",
                _ => "  "
            };
            println!("{} {}: {}", marker, i, v.display().to_string());
        }
    }
}

fn main() {
    let mut state = State::new("state.ini").unwrap_or_else(|err| {
        println!("unable to process state: {:#?}", err);
        process::exit(1);
    });

    let playlist = Playlist::new("files.txt", &state).unwrap_or_else(|err| {
        println!("unable to process playlist: {:#?}", err);
        process::exit(1);
    });

    let server = tiny_http::Server::http("0.0.0.0:8000").unwrap();

    let mut child:Option::<Child> = None;
    let mut current_track = playlist.next(&mut state);
    loop {
        match server.recv_timeout(time::Duration::from_millis(500)) {
            Ok(Some(rq)) => {
                let reply = match rq.url() {
                    "/skip" => {
                        match &mut child {
                            Some(c) => {
                                c.kill();
                                c.wait();
                                "<html><head><meta http-equiv=\"refresh\" content=\"0; url=/\"/></head></html>".to_string()
                            },
                            _ => "no track playing".to_string()
                        }
                    },
                    "/" => format!("current track: {}<br/><a href=\"/skip\">skip</a>", current_track.display()),
                    _ => "supported request".to_string()
                };
                let response = tiny_http::Response::from_data(reply.to_string().into_bytes());
                let response = response.with_header(
                    tiny_http::Header{ field: "Content-Type".parse().unwrap(), value: "text/html".parse().unwrap() }
                );
                rq.respond(response);
            },
            Err(e) => { println!("http error {}", e); },
            _ => {}
        }

        match &mut child {
            None => {
                current_track = playlist.next(&mut state);
                println!("track {}", current_track.display());
                match Command::new("/bin/sleep").arg("30").spawn() {
                    Ok(c) => child = Some(c),
                    Err(e) => println!("unable to start playing: { }", e.to_string())
                }
            },
            Some(c) => {
                match c.try_wait() {
                    Ok(Some(_)) => {
                        /* should we care about the status here? */
                        child = None;
                    },
                    Ok(None) => { /* no updates yet */ },
                    Err(e) => {
                        println!("unable to check player state: {}", e.to_string())
                    }
                }
            }
        }
    }

    state.write("state.ini");
}
