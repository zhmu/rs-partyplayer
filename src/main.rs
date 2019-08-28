use std::io::{ self, BufRead, BufReader };
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{ self, Child, Command };
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

struct Player<'a, 'b> {
    playlist: &'a Playlist,
    state: &'b mut State,
    child: Option::<Child>,
    current_track: Option::<&'a PathBuf>
}

impl<'a, 'b> Player<'a, 'b> {
    fn new(playlist: &'a Playlist, state: &'b mut State) -> Player<'a, 'b> {
        Player{ playlist, state, child: None, current_track: None }
    }

    fn poll_child(&mut self) {
        match &mut self.child {
            None => {
                self.current_track = Some(self.playlist.next(&mut self.state));
                match self.state.write("state.ini") {
                    Ok(()) => {},
                    Err(e) => println!("unable to save state: {}", e.to_string())
                }

                println!("track {}", self.current_track.unwrap().display());
                match Command::new("/bin/sleep").arg("30").spawn() {
                    Ok(c) => self.child = Some(c),
                    Err(e) => println!("unable to start playing: {}", e.to_string())
                }
            },
            Some(c) => {
                match c.try_wait() {
                    Ok(Some(_)) => {
                        /* should we care about the status here? */
                        self.child = None;
                    },
                    Ok(None) => { /* no updates yet */ },
                    Err(e) => {
                        println!("unable to check player state: {}", e.to_string())
                    }
                }
            }
        }
    }

    fn skip(&mut self) -> io::Result<()> {
        match &mut self.child {
            Some(c) => {
                c.kill()?;
                c.wait()?;
                Ok(())
            },
            _ => Ok(()) // nothing is playing; we should pick up a new track soon
        }
    }
}

fn poll_http_server(server: &tiny_http::Server, player: &mut Player) -> io::Result<()> {
    if let Some(rq) = server.recv_timeout(time::Duration::from_millis(500))? {
        let reply = match rq.url() {
            "/skip" => {
                match player.skip() {
                    Err(e) => format!("unable to skip track: {}", e),
                    _ => "<html><head><meta http-equiv=\"refresh\" content=\"0; url=/\"/></head></html>".to_string()
                }
            },
            "/" => format!("current track: {}<br/><a href=\"/skip\">skip</a>", player.current_track.unwrap().display()),
            _ => "supported request".to_string()
        };
        let response = tiny_http::Response::from_data(reply.to_string().into_bytes());
        let response = response.with_header(
            tiny_http::Header{ field: "Content-Type".parse().unwrap(), value: "text/html".parse().unwrap() }
        );
        return rq.respond(response)
    } Ok(())
}

fn main() {
    let mut state = State::new();
    if Path::new("state.ini").exists() {
        if let Err(err) = state.load("state.ini") {
            println!("unable to process state: {:#?}", err);
            process::exit(1);
        }
    }

    let playlist = Playlist::new("files.txt", &state).unwrap_or_else(|err| {
        println!("unable to process playlist: {:#?}", err);
        process::exit(1);
    });

    let server = tiny_http::Server::http("0.0.0.0:8000").unwrap_or_else(|err| {
        println!("unable to start http server: {:#?}", err);
        process::exit(1);
    });

    let mut player = Player::new(&playlist, &mut state);
    loop {
        if let Err(e) = poll_http_server(&server, &mut player) {
            println!("error from http server: {}", e)
        }
        player.poll_child();
    }
}
