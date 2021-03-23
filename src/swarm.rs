//! This module contains the functionality to connect to the 'swarm' of a channel.
//! The swarm is a P2P network that facilitates the sharing of data and events.

use std::{
	convert::TryInto,
	fmt,
	str::Utf8Error,
	sync::{
		atomic::*,
		Arc
	}
};

use async_std::{
	sync::Mutex
};
use bincode;
use gnunet::{
	cadet,
	crypto::HashCode,
	identity::PublicKey
};
use lazy_static::lazy_static;
use serde::*;
use unsafe_send_sync::UnsafeSend;

use crate::{
	common::*,
	event::*,
	message::*,
	persistence::{self, channel},
	runtime,
	session_manager::SessionManager
};



pub struct BadPeerStore {
	peers: Vec<PublicKey>
}

#[derive(Debug)]
pub enum Error {
	MessageMalformed( MessageMalformedError ),
	Gnunet( gnunet::Error ),
	Persistence( persistence::Error ),
	Internal( Box<dyn std::error::Error> )
}

#[derive(Debug)]
pub enum MessageMalformedError {
	/// When a deserialization with bincode failed.
	DeserializationIssue( bincode::Error, String ),
	/// When expecting a boolean but something other than a 1 or 0 was given.
	InvalidBoolean( u8, String ),
	InvalidHash( String ),
	InvalidSignature( String ),
	/// When some sort of type is give as a byte, and that byte uses an unknown ID.
	InvalidTypeId( u8, String ),
	/// When reading a string failed.
	InvalidUtf8( Utf8Error, String ),
	/// When a event message appeared to be way to new.
	InvalidEventId( u64 ),
	/// When the message turns out to be too small for the data is should contain.
	MissingData( String ),
	UnknownPublisher( PublicKey )
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Node ( Arc<NodeInner> );

struct NodeInner {
	pub connected: AtomicBool,
	pub persistence: UnsafeSend<channel::Handle>,	// TODO: Find out why channel::Handle is not send...
	pub parent_address: PublicKey,
	pub relay_power: u8,
	pub parent_socket: Mutex<cadet::Channel>,
	pub child_sockets: Vec<Mutex<cadet::Channel>>,
	session_manager: Mutex<SessionManager>,
	latest_event_id: Mutex<u64>
}



lazy_static! {
	pub static ref QUARTZ_PORT: HashCode = HashCode::generate( "QuartzNet".as_bytes() );
}



impl Node {

	/// Connects to another node of the swarm that is open to accept child nodes.
	/// 
	/// # Arguments
	/// `parent_address` - The address of the parent node to connect to.
	/// `relay_power` - The number of child peers this node is accepting.
	pub async fn connect( persistence: channel::Handle, cadet_handle: Arc<Mutex<cadet::Handle>>, parent_address: PublicKey, relay_power: u8 ) -> Result<Self> {

		let latest_event_id = persistence.get_latest_id("event").await?.expect("latest event id not found");

		let parent_socket = cadet_handle.lock().await.channel_connect( &parent_address, &QUARTZ_PORT ).await
			.map_err(|e| Error::Gnunet(e.into()))?;
		
		let inner = Arc::new( NodeInner {
			connected: true.into(),
			persistence: UnsafeSend::new( persistence ),
			parent_address: parent_address.clone(),
			relay_power,
			parent_socket: Mutex::new( parent_socket ),
			child_sockets: Vec::with_capacity( relay_power as _ ),
			session_manager: Mutex::new( SessionManager::new() ),
			latest_event_id: Mutex::new( latest_event_id )
		});

		// Runs the receive loop for the parent peer
		let inner2 = inner.clone();
		
		runtime::spawn(async move {
			Node::parent_receive_loop( inner2, |peer| {
				eprintln!("Peer {} is considered bad.", peer)
			}, |e| {
				eprintln!("Error occurred while listening to parent peer {}: {}", parent_address, e)
			} );
		});
		

		Ok( Self (
			inner
		))
	}

	pub async fn disconnect( &self ) {
		// TODO: Notify children about disconnection, which gives them your parent node.
		//       This way they don't have to reconnect to the network.
		// TODO: Maybe make this non-async.

		for child in &self.0.child_sockets {
			let _ = child.lock().await.destroy().await;
		}

		let _ = self.0.parent_socket.lock().await.destroy().await;
	}

	async fn parent_receive_loop<F,E>( this: Arc<NodeInner>, on_bad_peer: F, on_error: E ) where
		F: Fn( &PublicKey ),
		E: Fn( gnunet::Error )
	{
		Self::peer_receive_loop( this.clone(), &this.parent_address, &this.parent_socket, on_bad_peer, on_error ).await;
	}

	/// The loop that needs to be run in order to process the messages that this node may receive for a given peer
	/// 
	/// # Arguments
	/// `on_bad_peer` - A closure that is called whenever it is identified that the given peer is malicious.
	///                 This can have multiple reasons. Most often it is because the message has appeared incorrect.
	async fn peer_receive_loop<F,E>( this_: Arc<NodeInner>, address: &PublicKey, channel: &Mutex<cadet::Channel>, on_bad_peer: F, on_error: E ) where
		F: Fn( &PublicKey ),
		E: Fn( gnunet::Error )
	{
		// Loop until channel is closed
		loop {
			let this = this_.clone();
			let receiver = channel.lock().await.clone_receiver();
			let result: gnunet::Result<bool> = async {

				let message = match receiver.receive().await {
					None => return Ok(false),	// break
					Some(m) => m
				};
				match Self::process_message( this, &channel, &*message.payload, &on_error ).await {
					Err(err) => {
						match err {
							Error::MessageMalformed(e) => {
								eprintln!("Malformed message received from peer: {}, repelling it...", e);
								on_bad_peer( &address );
								return Ok(false)	// break
							},
							Error::Gnunet(e) => Err(e)?,
							other => panic!("Unexpected error occurred while processing message: {}", other)
						}
					},
					Ok(()) => Ok(true)
				}
			}.await;

			match result {
				Err(e) => on_error( e ),
				Ok(cont) => if !cont { break }
			}
		}
	}

	/// Processes a message from a peer.
	/// Returns whether or not the message was considered to be benevolent.
	/// If the message was malformed, the message is considered to be malicious.
	/// When something goes wrong during relaying a message to other peers, `on_error` is called with the error.
	/// This is because those errors shouldn't impede control flow.
	async fn process_message<E>( this: Arc<NodeInner>, channel: &Mutex<cadet::Channel>, message: &[u8], on_error: &E ) -> Result<()> where
		E: Fn(gnunet::Error)
	{

		let direction_type: MessageDirectionType = match message[0].try_into() {
			Err(e) => Err(MessageMalformedError::InvalidTypeId(message[0], "direction type".to_owned()))?,
			Ok(d) => d
		};
		
		match direction_type {
			MessageDirectionType::Event => Self::process_event( this.clone(), channel, &message[1..], on_error ).await?,
			MessageDirectionType::Request => Self::process_request( this, channel, &message[5..] ).await?,
			MessageDirectionType::Response => Self::process_response( this, &message[1..] ).await?
		};

		Ok(())
	}

	async fn process_event<E>( this: Arc<NodeInner>, channel: &Mutex<cadet::Channel>, message: &[u8], on_error: &E ) -> Result<()> where
		E: Fn(gnunet::Error)
	{

		let id: u64 = bincode::deserialize( message )?;

		let event_type: EventType = bincode::deserialize( &message[8..] )?;

		{
			let mut latest_event_id = this.latest_event_id.lock().await;

			// If this is the next event we need to process, process it immediately.
			if id == (*latest_event_id + 1) {

				match event_type {
					EventType::Channel => Self::process_event_channel( this.clone(), id, &message[1..] ).await?,
					EventType::Publisher( address ) => Self::process_event_publisher( this.clone(), id, &address, &message[1..] ).await?
				}

				// We can only update the event id after we know it wasn't malformed
				*latest_event_id = id;
			}
			// Otherwise, store it for later processing
			else {
				// If the event is that much more newer than our last received/known event id,
				//  we assume the message was malevolent.
				// Otherwise, we'd be vurnerable to filling our disk with senseless data.
				if id - *latest_event_id > 100 {
					Err( MessageMalformedError::InvalidEventId( id ) )?
				}
				if id > *latest_event_id {
					let start = 8 + bincode::serialized_size( &event_type ).unwrap() as usize;
					let event_message = &message[start..];

					match event_type {
						EventType::Channel => this.persistence.store_event( id, event_message ).await?,
						EventType::Publisher(address) => match this.persistence.get_timeline( &address ).await? {
							None => Err( MessageMalformedError::UnknownPublisher(address) )?,
							Some( timeline ) => timeline.store_event( id, event_message ).await?
						}
					}
				}
			}
		}

		// Either way, rebroadcast the message if the event wasn't found to be malformed/invalid.
		Self::rebroadcast_message( this, message, channel.lock().await.id(), on_error ).await;

		Ok(())
	}

	async fn process_event_channel( this: Arc<NodeInner>, id: u64, message: &[u8] ) -> Result<()> {
		if message.len() == 0 {
			Err(MessageMalformedError::MissingData("channel event".to_owned()))?
		}

		let event_type: ChannelEventType = match message[0].try_into() {
			Err(_) => Err(MessageMalformedError::InvalidTypeId(message[0], "channel event type".to_owned()))?,
			Ok(e) => e
		};

		match event_type {
			ChannelEventType::UpdateChannelProfile => Self::process_event_channel_update_profile( this, id, &message[1..] ).await,
			ChannelEventType::UpdatePublisherList => Self::process_event_channel_update_publisher_list( this, id, &message[1..] ).await
		}
	}

	async fn process_event_channel_update_profile( this: Arc<NodeInner>, id: u64, message: &[u8] ) -> Result<()> {

		let msg: UpdateChannelProfileEventMessage = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "upgrade profile event message".to_owned()))?;
		
		let mut profile_data = bincode::serialize( &msg.profile ).expect("unable to serialize channel profile");
		let mut checksum_content = bincode::serialize( &id ).unwrap();
		checksum_content.append( &mut profile_data );

		// Check hash
		if HashCode::generate_from( &msg.profile ) != msg.hash {
			Err(MessageMalformedError::InvalidHash("upgrade profile event message".to_owned()))?;
		}

		// Check signature
		let public_key = this.persistence.load_address().await?;
		if !msg.signature.verify_hash( &msg.hash, &public_key ) {
			Err(MessageMalformedError::InvalidSignature("upgrade profile event message".to_owned()))?
		}

		let current_profile = this.persistence.fetch_profile().await?;

		// If new, store it
		if current_profile.is_none() || msg.profile.base.revision > current_profile.unwrap().base.revision {
			this.persistence.store_profile( &msg.profile ).await?;
		}

		Ok(())
	}

	async fn process_event_channel_update_publisher_list( this: Arc<NodeInner>, event_id: u64, message: &[u8] ) -> Result<()> {

		let publisher_address: Vec<PublicKey> = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "publisher address".to_owned()))?;

		// TODO: Update the publisher list

		Ok(())
	}

	async fn process_event_publisher( this: Arc<NodeInner>, event_id: u64, address: &PublicKey, message: &[u8] ) -> Result<()> {
		let mut step = 33usize;	// Size of the public key in bytes

		if message.len() < (step + 1) {
			Err(MessageMalformedError::MissingData("publisher event".to_owned()))?
		}

		let event_type: PublisherEventType = match message[step].try_into() {
			Err(_) => Err(MessageMalformedError::InvalidTypeId(message[step], "publisher event type".to_owned()))?,
			Ok(e) => e
		};
		step += 1;

		match event_type {
			PublisherEventType::UpdateProfile => Self::process_event_publisher_update_profile( this, &address, &message[step..] ).await,
			PublisherEventType::PublishPost => Self::process_event_publisher_publish_post( this, &address, &message[step..] ).await,
			PublisherEventType::RevisePost => Self::process_event_publisher_revise_post( this, &address, &message[step..] ).await,
			PublisherEventType::ForgetPost => Self::process_event_publisher_forget_post( this, &address, &message[step..] ).await
		}
	}

	async fn process_event_publisher_forget_post( this: Arc<NodeInner>, publisher: &PublicKey, message: &[u8] ) -> Result<()> {

		let post_id: u64 = match bincode::deserialize( message ) {
			Err(e) => Err(MessageMalformedError::DeserializationIssue(e, "publisher event post id".to_owned()))?,
			Ok(id) => id
		};

		// TODO: Store the post

		Ok(())
	}

	async fn process_event_publisher_publish_post( this: Arc<NodeInner>, publisher: &PublicKey, message: &[u8] ) -> Result<()> {

		let post_id: u64 = match bincode::deserialize( message ) {
			Err(e) => Err(MessageMalformedError::DeserializationIssue(e, "publisher event post id".to_owned()))?,
			Ok(id) => id
		};

		// TODO: Store the post

		Ok(())
	}

	async fn process_event_publisher_revise_post( this: Arc<NodeInner>, publisher: &PublicKey, message: &[u8] ) -> Result<()> {

		let post_id: u64 = match bincode::deserialize( message ) {
			Err(e) => Err(MessageMalformedError::DeserializationIssue(e, "publisher event post id".to_owned()))?,
			Ok(id) => id
		};

		// TODO: Store the post

		Ok(())
	}

	async fn process_event_publisher_update_profile( this: Arc<NodeInner>, publisher: &PublicKey, message: &[u8] ) -> Result<()> {

		let profile: Profile = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "upgrade profile event profile".to_owned()))?;

		// TODO: Store the post

		Ok(())
	}

	async fn process_request( this: Arc<NodeInner>, channel: &Mutex<cadet::Channel>, message: &[u8] ) -> Result<()> {
		if message.len() < 5 {
			Err( MessageMalformedError::MissingData("request".to_owned()) )?;
		}

		let request_id: u32 = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "request id".to_owned()) )?;

		let request_type: RequestType = message[4].try_into()
			.map_err(|_| MessageMalformedError::InvalidTypeId(message[4], "request type".to_owned()))?;

		let (result_type, payload) = match request_type {
			RequestType::Posts => Self::process_request_posts( this.clone(), &message[5..] ).await?,
			RequestType::Files => { eprintln!("Files request not supported yet..."); return Ok(()) },
			RequestType::Blocks => { eprintln!("Blocks request not supported yet..."); return Ok(()) }
		};

		Self::respond( this, &mut *channel.lock().await, request_id, result_type, &*payload ).await?;

		Ok(())
	}

	async fn process_request_posts( this: Arc<NodeInner>, message: &[u8] ) -> Result<(ResponseResultType, Vec<u8>)> {
		#[derive(Deserialize)]
		struct Posts {
			timeline_id: PublicKey,
			post_id_start: u64,
			post_id_count: u16
		}

		/// Reads the nth bit of the given mask, where n = `index + 1`.
		fn get_bit( mask: &[u8], index: u16 ) -> bool {
			debug_assert!(index/8 < mask.len() as u16, "index out of bounds");
			let byte = mask[ (index / 8) as usize ];
			let byte_index = index % 8;
			byte & (1 << byte_index) > 0
		}

		/// Reads the nth bit of the given mask, where n = `index + 1`.
		fn set_bit( mask: &mut [u8], index: u16 ) {
			debug_assert!(index/8 < mask.len() as u16, "index out of bounds");
			let mut byte = 0u8;
			let byte_index = index % 8;
			byte |= 1 << byte_index;
			mask[ (index / 8) as usize ] = byte;
		}

		let Posts{timeline_id, post_id_start, post_id_count} = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "posts request".to_owned()))?;

		// The post id mask
		let next = 33 + 8 + 2;
		let mut mask_length: usize = (post_id_count / 8) as _;	if post_id_count % 8 > 0 { mask_length += 1 };
		let mask = &message[next..(next+mask_length)];
		let mut found_mask = vec!(0u8; mask.len());

		// Collect all available posts and update the 'found' mask.
		let mut posts = Vec::with_capacity( post_id_count as _ );
		let timeline = match this.persistence.get_timeline( &timeline_id ).await? {
			None => Err( MessageMalformedError::UnknownPublisher( timeline_id ) )?,
			Some(t) => t
		};
		for i in 0..post_id_count {
			let bit = get_bit( mask, i );

			let result = timeline.load_post( i as _ ).await?;
			if !result.is_none() {
				posts.push( result.unwrap() );
				set_bit( &mut *found_mask, i );
			}
		}

		return Ok(( ResponseResultType::Success, found_mask ))
	}

	async fn process_response( this: Arc<NodeInner>, message: &[u8] ) -> Result<()> {

		let session_id: u32 = bincode::deserialize( message )
			.map_err(|e| MessageMalformedError::DeserializationIssue(e, "response request id".to_owned()))?;

		this.session_manager.lock().await.respond(session_id, message.to_owned()).await;

		Ok(())
	}

	/// Rebroadcasts the given event message to the parent and children, except for the node which channel id is provided with `skip_channel_id`.
	/// It tries to give the message to everybody.
	/// This might mean that errors occur for multiple peers.
	/// Every error occurence invokes `on_error` with the error provided.
	async fn rebroadcast_message<E>( this: Arc<NodeInner>, message: &[u8], skip_channel_id: u32, on_error: E ) where
		E: Fn(gnunet::Error)
	{
		let mut complete_msg = Vec::<u8>::with_capacity( 1 + message.len() );
		complete_msg.push( MessageDirectionType::Event.into() );
		complete_msg.extend_from_slice( message );

		let mut psock = this.parent_socket.lock().await;
		if psock.id() != skip_channel_id {
			match psock.send( cadet::PRIORITY_PREFERENCES_BEST_EFFORT, message ).await {
				Err(e) => on_error(e.into()),
				Ok(()) => {}
			}
		}

		for child in this.child_sockets.iter() {
			let csock = child.lock().await;
			match psock.send( cadet::PRIORITY_PREFERENCES_BEST_EFFORT, message ).await {
				Err(e) => on_error(e.into()),
				Ok(()) => {}
			}
		}
	}

	async fn respond( this: Arc<NodeInner>, channel: &mut cadet::Channel, request_id: u32, result: ResponseResultType, response: &[u8] ) -> Result<()> {
		
		// TODO: Implement an interface in the gnunet::cadet::channel, that allows sending part by part,
		//        so we don't have to construct a message first.
		let mut message = Vec::with_capacity( 6 + response.len() );
		message.push( MessageDirectionType::Response as u8 );
		message.extend_from_slice( &request_id.to_le_bytes() );
		message.push( result as u8 );
		message.extend_from_slice( response );

		channel.send( cadet::PRIORITY_PREFERENCES_BEST_EFFORT, &*message ).await
			.map_err(|e| Error::Gnunet(e.into()))?;

		Ok(())
	}
}

impl fmt::Display for Error {
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result {
		match self {
			Self::MessageMalformed(e) => write!(f, "malformed message: {}", e),
			Self::Gnunet(e) => write!(f, "gnunet issue: {}", e),
			Self::Persistence(e) => write!(f, "persistence issue: {}", e),
			Self::Internal(e) => write!(f, "internal issue: {}", e)
		}
	}
}

impl std::error::Error for Error {
	fn source( &self ) -> Option<&(dyn std::error::Error + 'static)> {
		None
		/*Some( match self {
			Self::MessageMalformed(e) => e,
			Self::Gnunet(e) => e,
			Self::Internal(e) => &*e
		})*/
	}
}

impl From<gnunet::Error> for Error {
	fn from( other: gnunet::Error ) -> Self {
		Self::Gnunet(other)
	}
}

impl From<persistence::Error> for Error {
	fn from( other: persistence::Error ) -> Self {
		Self::Persistence(other)
	}
}

/// Care should be taken that deserialization errors from malformed messages don't get transformed into persistence errors.
impl From<bincode::Error> for Error {
	fn from( other: bincode::Error ) -> Self {
		Self::MessageMalformed(MessageMalformedError::DeserializationIssue(other, "unknown".to_owned()))
	}
}

impl From<MessageMalformedError> for Error {
	fn from( other: MessageMalformedError ) -> Self {
		Self::MessageMalformed(other)
	}
}

impl fmt::Display for MessageMalformedError {
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result {
		match self {
			Self::DeserializationIssue(e, desc) => write!(f, "deserialization issue while parsing {}: {}", desc, e),
			Self::InvalidBoolean(id, desc) => write!(f, "invalid boolean found for {}: {}", desc, id),
			Self::InvalidEventId(id) => write!(f, "invalid event ID: {}", id),
			Self::InvalidHash(desc) => write!(f, "invalid checksum for {}", desc),
			Self::InvalidSignature(desc) => write!(f, "signature verification failed for {}", desc),
			Self::InvalidTypeId(id, desc) => write!(f, "invalid type id found for {}: {}", desc, id),
			Self::InvalidUtf8(e, desc) => write!(f, "invalid UTF-8 for {}: {}", desc, e),
			Self::MissingData(desc) => write!(f, "missing data for {}", desc),
			Self::UnknownPublisher(address) => write!(f, "unknown publisher address: {}", address.to_string())
		}
	}
}

impl std::error::Error for MessageMalformedError {
	fn source( &self ) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::DeserializationIssue(e, _) => Some(e),
			Self::InvalidUtf8(e, _) => Some(e),
			other => None
		}
	}
}