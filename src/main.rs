use actix_web::{App, HttpServer};
use gnunet;
use tera::Tera;

use std::{
	sync::Arc
};



mod common;
mod config;
mod event;
mod r#macro;
mod message;
mod persistence;
mod post;
mod runtime;
mod session_manager;
mod subscriptions;
mod swarm;
mod web;



pub const RETURN_CODE_OK: i32 = 0;
pub const RETURN_CODE_UNEXPECTED: i32 = 1;



pub struct Globals {
	gnunet: gnunet::Handle,
	tera: tera::Tera
}



#[actix_web::main]
async fn main() {

	let gnunet = gnunet::Handle::default();
	let tera = Tera::new(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/**/*")).unwrap();
	let globals = Arc::new( Globals {
		gnunet,
		tera
	});

	let server = match HttpServer::new(move || {

		App::new()
			.data(globals.clone())
			.service(web::homepage)
			.service(web::channel_feed)
			.service(web::channel_feed_first)
			//.service(web::channel_feed_post)
			.service(web::channel_new)
			.service(web::channel_new_post)
	}).bind("0.0.0.0:7777") {
		Err(e) => { eprintln!("Unable to start HTTP server: {}", e); return },
		Ok(server) => server
	};
	eprintln!("HTTP server starting...");

	match server.run().await {
		Err(e) => eprintln!("HTTP server error: {}", e),
		Ok(()) => {}
	}

	eprintln!("HTTP server stopped.")
}