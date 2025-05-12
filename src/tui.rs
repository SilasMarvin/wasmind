use std::io::{self, Write};

pub fn display_user_prompt() {
    print!("\n(user) >>> ");
    io::stdout().flush().unwrap();
}

pub fn display_assistant_start() {
    print!("\n(assistant) >>> ");
}

pub fn display_screenshot(name: &str) {
    println!("\n|{}|", name);
}

pub fn display_clipboard_excerpt(content: &str) {
    // Take first 50 chars or up to first newline, whichever comes first
    let excerpt = content
        .lines()
        .next()
        .unwrap_or(content)
        .chars()
        .take(50)
        .collect::<String>();

    if content.len() > excerpt.len() {
        println!("\n[clipboard] {}...", excerpt);
    } else {
        println!("\n[clipboard] {}", excerpt);
    }
}

pub fn display_text(text: &str) {
    print!("{}", text);
    io::stdout().flush().unwrap();
}

pub fn display_error(error: &str) {
    eprintln!("\nError: {}", error);
}

pub fn display_done_marker() {
    println!("\n---");
}

pub fn clear_line() {
    print!("\r\x1B[2K");
    io::stdout().flush().unwrap();
}
