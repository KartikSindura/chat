use crossterm::{
    QueueableCommand,
    cursor::MoveTo,
    event::{Event, KeyCode, KeyModifiers, poll, read},
    terminal,
};
use std::{env, str};
use std::{
    io::{ErrorKind, Read, Write, stdout},
    time::Duration,
};
use std::{net::TcpStream, thread};

struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

fn chat_window(stdout: &mut impl Write, chat: &[String], boundary: Rect) {
    let n = chat.len();
    let m = n.checked_sub(boundary.h).unwrap_or(0);

    for (dy, line) in chat.iter().skip(m).enumerate() {
        stdout
            .queue(MoveTo(boundary.x as u16, (boundary.y + dy) as u16))
            .unwrap();
        let bytes = line.as_bytes();
        stdout
            .write_all(bytes.get(0..boundary.w).unwrap_or(bytes))
            .unwrap();
    }
}

fn main() {
    let mut args = env::args();
    let program = args.next().expect("program name");
    let ip = args.next().expect("provide ip");
    let port = args.next().expect("port");

    let mut stream = TcpStream::connect(format!("{ip}:{port}")).unwrap();
    stream.set_nonblocking(true).unwrap();

    let mut stdout = stdout();
    terminal::enable_raw_mode().unwrap();
    let (mut w, mut h) = terminal::size().unwrap();
    let mut bar = "-".repeat(w as usize);
    let mut prompt = String::new();
    let mut quit = false;
    let mut chat = Vec::new();
    let mut buf = [0; 64];
    while !quit {
        while poll(Duration::ZERO).unwrap() {
            match read().unwrap() {
                Event::Resize(nw, nh) => {
                    w = nw;
                    h = nh;
                    bar = "-".repeat(w as usize);
                }
                Event::Paste(data) => {
                    prompt.push_str(&data);
                }
                Event::Key(event) => match event.code {
                    KeyCode::Char(x) => {
                        if x == 'c' && event.modifiers.contains(KeyModifiers::CONTROL) {
                            quit = true;
                        } else {
                            prompt.push(x);
                        }
                    }
                    KeyCode::Enter => {
                        stream.write_all(prompt.as_bytes()).unwrap();
                        chat.push(prompt.clone());
                        prompt.clear();
                    }
                    KeyCode::Esc => {
                        prompt.clear();
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        match stream.read(&mut buf) {
            Ok(n) => {
                if n > 0 {
                    chat.push(str::from_utf8(&buf[0..n]).unwrap().to_string());
                } else {
                    quit = true;
                }
            }
            Err(err) => {
                if err.kind() != ErrorKind::WouldBlock {
                    panic!("{err}");
                }
            }
        }

        stdout
            .queue(terminal::Clear(terminal::ClearType::All))
            .unwrap();

        chat_window(
            &mut stdout,
            &chat,
            Rect {
                x: 0,
                y: 0,
                w: w as usize,
                h: h as usize - 2,
            },
        );

        stdout.queue(MoveTo(0, h - 2)).unwrap();
        stdout.write_all(bar.as_bytes()).unwrap();

        stdout.queue(MoveTo(0, h - 1)).unwrap();

        {
            let bytes = prompt.as_bytes();
            stdout
                .write_all(bytes.get(0..w as usize).unwrap_or(bytes))
                .unwrap();
        }

        stdout.flush().unwrap();
        thread::sleep(Duration::from_millis(30));
    }
    terminal::disable_raw_mode().unwrap();
}
