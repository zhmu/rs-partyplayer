use std::io::{ self, BufRead, BufReader };
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process;
use std::process::{ Child, Command };
use std::time;

use rand::{ Rng, SeedableRng };
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use ini::Ini;

extern crate tiny_http;

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
    fn new() -> State {
        // Generate a new random seed
        let mut rng = rand::thread_rng();
        let seed = rng.gen::<u64>();
        State { seed, index: 0 }
    }

    fn load(&mut self, path: &str) -> Result<(), StateError> {
        // INI file found; we'd better be able to parse it
        let conf = Ini::load_from_file(path).map_err(StateError::Ini)?;
        let section = conf.section(Some("general".to_owned())).unwrap();
        self.seed = section.get("seed").unwrap().parse::<u64>().map_err(StateError::ParseInt)?;
        self.index = section.get("index").unwrap().parse::<usize>().map_err(StateError::ParseInt)?;
        Ok(())
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
    files: Vec<PathBuf>
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

        Ok(Playlist{ files })
    }

    fn next(&self, state: &mut State) -> &PathBuf {
        let n = state.index;
        state.index += 1;
        &self.files[n]
    }
}

fn main() {
    let mut state = State::new();
    if Path::new("state.ini").exists() {
        match state.load("state.ini") {
            Err(err) => {
                println!("unable to process state: {:#?}", err);
                process::exit(1);
            },
            _ => { }
        }
    }

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
                                match c.kill().and_then(|()| c.wait()) {
                                    Err(e) => format!("failed: {}", e),
                                    _ => "<html><head><meta http-equiv=\"refresh\" content=\"0; url=/\"/></head></html>".to_string()
                                }
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
                let _ = rq.respond(response);
            },
            Err(e) => { println!("http error {}", e); },
            _ => {}
        }

        match &mut child {
            None => {
                current_track = playlist.next(&mut state);
                match state.write("state.ini") {
                    Ok(()) => {},
                    Err(e) => println!("unable to save state: {}", e.to_string())
                }

                println!("track {}", current_track.display());
                match Command::new("/bin/sleep").arg("30").spawn() {
                    Ok(c) => child = Some(c),
                    Err(e) => println!("unable to start playing: {}", e.to_string())
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
}
