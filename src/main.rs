use std::{ fs, io::{ prelude::* }, net::{ TcpListener, TcpStream } };
use colored::Colorize;

fn main() {
    let address = "127.0.0.1:7878";
    let listener = TcpListener::bind(address).unwrap();
    println!("{}{}", "Server started at: http://".green().bold(), address.green().bold());
    println!("Type {} to stop the server...", "CTRL+C".red().bold());

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        println!(
            "Connection established from {}",
            stream.peer_addr().unwrap().to_string().blue().bold()
        );

        handle_connection(stream);
    }

    println!("Shutting down server...");
}

fn handle_connection(mut stream: TcpStream) {
    let status_line = "HTTP/1.1 200 OK";
    let contents = get_file_list().join("\n");
    let length = contents.len();
    let response = format!("{status_line}\r\nContent-Length: {length}\r\n\r\n{contents}");
    stream.write_all(response.as_bytes()).unwrap();
}

fn get_file_list() -> Vec<String> {
    let mut file_list = Vec::new();
    let paths = fs::read_dir(".").unwrap();

    // List just the files in the current directory
    for path in paths {
        let path = path.unwrap().path();
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        file_list.push(format!("{file_name}"));
        // Don't list the executable file
        if file_name == "ls_web" {
            file_list.pop();
        }
    }

    // Add the total number of files
    let total_files = file_list.len();

    file_list.push(format!("Total files: {total_files:>4}"));

    file_list
}
