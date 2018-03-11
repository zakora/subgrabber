use flate2::read::GzDecoder;
use futures::{Future, Stream};
use hyper::{Client, Method, Request};
use hyper::header::{ContentLength, ContentType};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use regex::Regex;
use std::fs;
use std::io::prelude::*;
use std::str;
use tokio_core::reactor::Core;
use xdg;

// CLIENT
pub fn init_hyper() -> (Core, Client<HttpsConnector<HttpConnector>>) {
    // Create a Hyper client and tokio core that will be reused for the
    // different API calls to OpenSubtitles.
    let core = Core::new()
        .expect("failed to create tokio core");
    let handle = core.handle();

    let client = Client::configure()
        .connector(HttpsConnector::new(4, &handle)
                   .expect("failed to create HTTPS connector"))
        .build(&handle);

    (core, client)
}

// LOGIN / TOKEN
pub fn cached_token(core: &mut Core, client: &Client<HttpsConnector<HttpConnector>>) -> String {
    // Return an authentication token for the OpenSubtitles API.
    // If a token is found in the application cache then return it without
    // checking for its validaty. Otherwise, if not token is present in the
    // cache, ask for a token and store it in the cache.
    let xdg_dirs = xdg::BaseDirectories::with_prefix("subgrabber")
        .expect("failed to create XDG directory");
    let mut token = String::new();
    match xdg_dirs.find_cache_file("token") {
        Some(token_path) => {
            let mut token_file = fs::File::open(token_path)
                .expect("failed to open token file");
            token_file.read_to_string(&mut token)
                .expect("failed to read token file");
        },
        None => {
            token = req_token(core, &client);
            store_token(&token);
        }
    }

    token
}

pub fn store_token(token: &String) {
    // Write a token to disk in the application cache.

    let xdg_dirs = xdg::BaseDirectories::with_prefix("subgrabber")
        .expect("failed to create XDG directory");
    // Create token cache file
    let token_path = xdg_dirs.place_cache_file("token")
        .expect("failed to create token cache file");
    let mut token_file = fs::File::create(token_path)
        .expect("failed to open token cache file");

    // Write token to file
    write!(&mut token_file, "{}", token)
        .expect("failed to write to token cache file");
}

pub fn req_token(core: &mut Core, client: &Client<HttpsConnector<HttpConnector>>) -> String {
    // Call the OpenSubtitles API for a token.

    let payload = login_payload();
    let uri = "https://api.opensubtitles.org/xml-rpc".parse()
        .expect("failed to parse URI");

    // Prepare POST request
    let mut req = Request::new(Method::Post, uri);
    req.headers_mut().set(ContentType::xml());
    req.headers_mut().set(ContentLength(payload.len() as u64));
    req.set_body(payload);

    // Make the request
    let work = client.request(req).and_then(|res| {
        println!("Status: {}", res.status());
        res.body().concat2()
    });
    let posted = core.run(work).expect("failed to run tokio core work");

    let response = String::from_utf8(posted.to_vec())
        .expect("failed to convert utf8 to String");

    parse_token(response)
}

fn parse_token(response: String) -> String {
    // Parse the XML reponse from the OpenSubtitles API to return the token
    // value.

    let re = Regex::new(r"<name>token</name><value><string>([[:ascii:]]+?)</string>")
        .expect("failed to parse token regex");
    let captures = re.captures(&response)
        .expect("failed to capture token using regex");
    captures[1].to_string()
}

fn login_payload() -> &'static str {
    // XML payload to send on a login call.
    r#"<?xml version="1.0"?>
    <methodCall>
        <methodName>LogIn</methodName>
        <params>
            <param>
            <value><string></string></value>
            </param>
            <param>
            <value><string></string></value>
            </param>
            <param>
            <value><string></string></value>
            </param>
            <param>
            <value><string>TemporaryUserAgent</string></value>
            </param>
        </params>
    </methodCall>
    "#
}

// SEARCH
fn search_payload(token: &mut String, hash: &String, size: u64) -> String {
    // XML payload to send on a search call, with placeholders for API parameters.
    format!(r#"<?xml version="1.0"?>
    <methodCall>
        <methodName>SearchSubtitles</methodName>
        <params>
            <param>
                <value><string>{}</string></value>
            </param>
            <param>
                <value>
                    <array>
                        <data>
                            <value>
                                <struct>
                                    <member>
                                        <name>sublanguageid</name>
                                        <value><string>eng</string></value>
                                    </member>
                                    <member>
                                        <name>moviehash</name>
                                        <value><string>{}</string></value>
                                    </member>
                                    <member>
                                        <name>moviebytesize</name>
                                        <value><string>{}</string></value>
                                    </member>
                                </struct>
                            </value>
                        </data>
                    </array>
                </value>
            </param>
        </params>
    </methodCall>
    "#, token, hash, size)
}

pub fn req_search(core: &mut Core, client: &Client<HttpsConnector<HttpConnector>>,
              token: &mut String, hash: &String, size: u64) -> Option<String> {
    // Do an API call to search for a subtitle given a computed file hash.
    // This may fail if the is token has expired of the API is down.

    let payload = search_payload(token, hash, size);
    let uri = "https://api.opensubtitles.org/xml-rpc".parse()
        .expect("failed to parse URI");

    // Prepare POST request
    let mut req = Request::new(Method::Post, uri);
    req.headers_mut().set(ContentType::xml());
    req.headers_mut().set(ContentLength(payload.len() as u64));
    req.set_body(payload);

    // Make the request
    let work = client.request(req).and_then(|res| {
        println!("search status: {}", res.status());
        res.body().concat2()
    });
    let posted = core.run(work)
        .expect("failed to do a search request");

    let response = String::from_utf8(posted.to_vec())
        .expect("failed to convert utf8 to String");

    parse_first_link(response)
}

fn parse_first_link(response: String) -> Option<String> {
    // Parse the API response for a subtitle link.
    // The API can return multiple subtitle link, in this case the first one is
    // returned because they are sorted by best match.

    let re = Regex::new(r"<name>SubDownloadLink</name><value><string>(.+?)</string></value>")
        .expect("failed to parse link regex");
    match re.captures(&response) {
        Some(captures) => Some(captures[1].to_string()),
        None => None
    }
}

// DOWNLOAD
pub fn req_download(core: &mut Core, client: &Client<HttpsConnector<HttpConnector>>,
                    sub_uri: String, sub_path: &String) {
    // Do an API call to download the gziped subtitle.
    // Decompress the subtitle and put it next to the video.

    // Make request to get the gzipped subtitle content
    let uri = sub_uri.parse().expect("failed to parse URI");
    let work = client.get(uri).and_then(|res| {
        res.body().concat2()
    });
    let resp = core.run(work).expect("failed to run tokio core work");

    // Gunzip the data
    let buffer = resp.to_vec();
    let mut data = GzDecoder::new(buffer.as_slice());
    let mut uncompressed = Vec::new();
    data.read_to_end(&mut uncompressed)
        .expect("failed to decompress data");

    // Write the uncompressed data a subtitle file
    let mut file_out = fs::File::create(sub_path)
        .expect("failed to create srt file");
    file_out.write_all(&uncompressed)
        .expect("could not write to srt file");
}
