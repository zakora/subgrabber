extern crate flate2;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate regex;
extern crate tokio_core;
extern crate xdg;

use std::env;
use std::fs;
use regex::Regex;

mod hash;
mod osapi;

fn remove_extension(path: &String) -> String {
    // Remove the extension in a file name.
    let re = Regex::new(r"^(.*)\..*$")
        .expect("failed to parse regex");
    let captures = re.captures(&path)
        .expect("failed regex capture");
    captures[1].to_string()
}

fn sub_exists(sub_path: &String) -> bool {
    // Return true if the file path exists, false otherwise.
    match fs::metadata(sub_path) {
        Ok(_) => true,
        Err(_) => false
    }
}

fn main() {
    // File path of a movie to get the subtitles for
    let args: Vec<String> = env::args().collect();
    let file_path = &args[1];
    let canonical_name = remove_extension(&file_path);

    // Check if subtitle already exists
    let sub_path = format!("{}.srt", canonical_name);
    if sub_exists(&sub_path) {
        println!("sub file already exists, skipping.");
        return ()
    }

    // Compute the OpenSubtitles file hash
    let (hash, size) = hash::compute(file_path);
    println!("hash: {}, size: {} bytes", hash, size);

    // Create a Hyper client and a tokio core, we will pass them around
    let (mut core, client) = osapi::init_hyper();

    // Download subtitle through OpenSubtitles API
    println!("Searching for matching subtitle on OpenSubtitles.org");
    let mut token = osapi::cached_token(&mut core, &client);
    let sub_link = match osapi::req_search(&mut core, &client, &mut token, &hash, size) {
        // If the search request succeeded, return the matching link.
        Some(link) => link,
        None => {
            // If the search failed, it is probably because the authentication token
            // has expired, so we renew it.
            println!("failed to do search, renewing the token and attempting the search again");
            token = osapi::req_token(&mut core, &client);
            osapi::store_token(&token);
            osapi::req_search(&mut core, &client, &mut token, &hash, size)
                .expect("failed to do a search with a fresh token, quitting.")
        }
    };

    // Download the subtitle title and extract it next to the video
    println!("Downloading the subtitle to {}", sub_path);
    osapi::req_download(&mut core, &client, sub_link, &sub_path);
}
