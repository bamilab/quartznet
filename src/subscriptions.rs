use std::{
	io,
	sync::Arc
};

use async_std::{
	fs::File,
	prelude::*,
	sync::Mutex
};

use gnunet::{
	cadet,
	identity::PublicKey
};
use serde::*;

use crate::{
	config,
	persistence::{
		self,
		channel,
		DATABASE_DIR
	},
	swarm::{self, Node}
};



#[derive(Debug)]
pub enum Error {
	
}

#[derive(Deserialize, Serialize)]
pub struct Subscription {
	/// The address of the owner of the channel.
	/// This key also identifies the channel.
	owner: PublicKey,
	/// The set of publishers that are known for this channel
	publishers: Vec<PublicKey>,
	/// Some extra peers that were remembered from the last session.
	/// The idea is that if the owner and the publishers are not online, these peers can be tried connecting to to find a connection back to the swarm.
	cached_peers: Vec<PublicKey>
}

pub struct SubscriptionManager {
	persistence: channel::Handle,
	pub sub: Subscription,
	node: Option<Node>
}

pub struct SubscriptionsManager {
	persistence: persistence::Handle,
	subs: Vec<SubscriptionManager>
}



impl Subscription {

	/// Attempts to find a connection to the swarm through any of the peers that it knows.
	/// If it fails to connect to a peer, its error is given through `on_error`.
	/// If connection could be made, `None` is returned.
	/// 
	/// # Arguments
	/// `cadet` - A handle to the Gnunet cadet service.
	/// `relay_power` - The power of the number of child peers our peer will accept.
	///                 So the number of accepted child peers is 2 to the power of `relay_power`.
	///                 Generally speaking, you want to default to 1.
	///                 If you want to provide a lot of bandwidth to the network, you can use very high numbers, and this will reduce latency in the network.
	pub async fn find_swarm_connection( &self, persistence: channel::Handle, cadet: Arc<Mutex<cadet::Handle>>, relay_power: u8, on_error: impl Fn( &PublicKey, swarm::Error ) ) -> Option<Node> {

		// First try some cached peer, so as to not overload the publisher nodes.
		for peer in &self.cached_peers {
			match Node::connect( persistence.clone(), cadet.clone(), peer.clone(), relay_power ).await {
				Err(e) => on_error(&peer, e),
				Ok(node) => return Some(node)
			}
		}

		// Then try the publishers, so asto not overload the owner node.
		for peer in &self.publishers {
			match Node::connect( persistence.clone(), cadet.clone(), peer.clone(), relay_power ).await {
				Err(e) => on_error(&peer, e),
				Ok(node) => return Some(node)
			}
		}

		// Then as a last resort, we try the owner node.
		match Node::connect( persistence.clone(), cadet.clone(), self.owner.clone(), relay_power ).await {
			Err(e) => on_error(&self.owner, e),
			Ok(node) => return Some(node)
		}

		// If that doesn't work, try to find an available peer node from the DHT.
		// TODO: Search the DHT for an address, and then connect to that.

		None
	}
}

impl SubscriptionManager {

	/// Loads the subscription manager for channel with given `address`.
	/// The subscription manager holds a live connection to the swarm.
	/// If no such connection could be made, the subscription manager automatically retries to attempt a connection every so often.
	pub async fn load( persistence: channel::Handle, cadet: Arc<Mutex<cadet::Handle>>, address: PublicKey ) -> persistence::Result<Self> {
		
		let sub = match File::open( DATABASE_DIR.join("subscriptions").join( address.to_string() ) ).await {
			Err(e) => {
				if e.kind() == io::ErrorKind::NotFound {
					Subscription {
						owner: address,
						publishers: Vec::new(),
						cached_peers: Vec::new()
					}
				}
				else {
					panic!("unable to open subscription file: {}", e)
				}
			},
			Ok(mut file) => {
				let mut content = Vec::new();
				file.read_to_end( &mut content ).await.expect("I/O error while reading subscription file");
				bincode::deserialize( &*content ).expect("deserialization error while reading subscription file")
			}
		};

		let node = sub.find_swarm_connection( persistence.clone(), cadet, config::RELAY_POWER, |a,e| {
			eprintln!("Unable to connect to peer {}: {}. Trying next...", a, e);
		}).await;

		Ok( Self {
			persistence,
			sub,
			node
		})
	}

	pub async fn save( &self ) -> io::Result<()> {

		let content = bincode::serialize( &self.sub ).expect("serialization error");
		let mut file = File::create( DATABASE_DIR.join("subscriptions").join( self.sub.owner.to_string() ) ).await?;
		file.write( &*content ).await?;

		Ok(())
	}
}

impl SubscriptionsManager {

	pub async fn load( persistence: persistence::Handle, cadet: cadet::Handle ) -> persistence::Result<Self> {

		let channels = persistence.list_channels().await?;
		let mut subs = Vec::with_capacity( channels.len() );
		let cadet_shared = Arc::new( Mutex::new( cadet ) );

		for channel in channels {
			subs.push(
				SubscriptionManager::load( channel.clone(), cadet_shared.clone(), channel.load_address().await? ).await?
			);
		}

		Ok( Self {
			persistence,
			subs
		})
	}

	pub async fn save() -> io::Result<()> {

		// TODO: Save the list of subscriptions

		Ok(())
	}
}