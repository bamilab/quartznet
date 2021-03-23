//! The session manager is responsible for connecting responses to requests.

use std::{
	collections::HashMap,
	time::Duration
};

use async_std::{
	channel::*,
	future::timeout
};



pub const SESSION_TIMEOUT: u64 = 10000;



pub struct SessionManager {
	sessions: HashMap<u32, SessionData>
}

struct SessionData {
	tx: Sender<Vec<u8>>
}



impl SessionManager {

	pub fn new() -> Self {
		Self {
			sessions: HashMap::new()
		}
	}

	/// Returns the message as a byte vector, or nothing if not response was received within the `SESSION_TIMEOUT`.
	pub async fn request( &mut self, session_id: u32 ) -> Option<Vec<u8>> {
		let (tx, rx) = bounded( 1 );

		self.sessions.insert( session_id, SessionData {tx});

		match timeout( Duration::from_millis(SESSION_TIMEOUT), rx.recv() ).await {
			Err(_) => None,
			Ok(r) => Some( r.expect("channel closed") )
		}
	}

	/// Provides the response message that will be relayed to the requester.
	/// Returns whether or not the session (still) existed.
	pub async fn respond( &mut self, session_id: u32, message: Vec<u8> ) -> bool {

		match self.sessions.remove( &session_id ) {
			None => false,
			Some( session_data ) => {

				session_data.tx.send(message).await.expect("channel closed");
				session_data.tx.close();

				true
			}
		}
	}
}