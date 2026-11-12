use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub(crate) type Log = Arc<Mutex<Vec<String>>>;

/// One thread per connection, so a client that keeps its connection open doesn't hold
/// up another (requests at once).
pub(crate) fn serve(
    route: impl Fn(&str, &str, &str) -> (u16, String) + Send + Sync + 'static,
) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let log2 = log.clone();
    let route = Arc::new(route);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (log, route) = (log2.clone(), route.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("").to_string();
                    let mut len = 0;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':')
                            && k.eq_ignore_ascii_case("content-length")
                        {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0; len];
                    if reader.read_exact(&mut body).is_err() {
                        break;
                    }
                    log.lock().unwrap().push(format!("{method} {path}"));
                    let (status, answer) = route(&method, &path, &String::from_utf8_lossy(&body));
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n",
                        answer.len()
                    );
                    if stream.write_all(head.as_bytes()).is_err()
                        || stream.write_all(answer.as_bytes()).is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    (base, log)
}

/// Like `serve`, with binary answers, and requests read as GETs.
pub(crate) fn serve_bytes(
    route: impl Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static,
) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let log2 = log.clone();
    let route = Arc::new(route);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (log, route) = (log2.clone(), route.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                    }
                    log.lock().unwrap().push(format!("GET {path}"));
                    let (status, answer) = route(&path);
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n",
                        answer.len()
                    );
                    if stream.write_all(head.as_bytes()).is_err()
                        || stream.write_all(&answer).is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
    (base, log)
}
