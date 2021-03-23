use std::fmt;

use gnunet::{
	crypto::HashCode,
	identity::PublicKey
};
use serde::{*, ser::SerializeTuple};

use crate::byte_enum;



pub enum EventType {
	Channel,
	Publisher( PublicKey )
}
struct EventTypeVisitor;

byte_enum! {
	pub enum ChannelEventType {
		/// Contains the new profile information
		UpdateChannelProfile = 0,
		UpdatePublisherList = 1
	}
}

byte_enum! {
	pub enum PublisherEventType {
		UpdateProfile = 0,
		/// Publishes a post and the hashcode that identifies the content of the post.
		PublishPost = 1,
		/// Changes the hash code that identifies a post, and provides the diffs that change the previous state of the post to the new one.
		RevisePost = 2,
		/// Requests the participating nodes to 'forget' a post.
		ForgetPost = 3
	}
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PublishPostEventData {
	/// The hash code taken from all hash codes of the blocks that make up the content.
	hash: HashCode
}

#[derive(Clone, Deserialize, Serialize)]
pub struct RevisePostEventData {
	old_post_id: u64,
	new_hash: HashCode,
	//diffs: Vec<Diff>
}

/// This is always the first event for the channel timeline.
/// This even message contains the parameters that define some settings of the channel.
/// These parameters can't be changed because if a publisher doesn't notice that change in its UI, 
///  the publisher would not know its data might be shared with more (or less) people than initially anticipated.
#[derive(Clone, Deserialize, Serialize)]
pub struct ChannelCreateEventData {

	/// If true, the posts from the listed publishers can be shared with any subscriber peer.
	pub public: bool,
	/// The number of days that the content is requested to be replicated by the subscribers.
	/// This is by no means a way to ensure a minimum or maximum replication time,
	///  but it can be used to keep posts not stored for too long in channels that are intended for a more temporal way of shared messages/content,
	///  like channels with the intention of being a chat room.
	pub requested_replication_time: u32
}



impl Serialize for EventType {

	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where
		S: Serializer
	{
		let mut s = serializer.serialize_tuple(2)?;

		match self {
			EventType::Channel => s.serialize_element(&0u8)?,
			EventType::Publisher(address) => {
				s.serialize_element(&1u8)?;
				s.serialize_element(address)?
			}
		}

		s.end()
	}
}

impl<'de> Deserialize<'de> for EventType {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where
		D: Deserializer<'de>
	{
		deserializer.deserialize_tuple( 0, EventTypeVisitor )
	}
}

impl<'de> de::Visitor<'de> for EventTypeVisitor {

	type Value = EventType;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("event type")
	}

	fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
		A: de::SeqAccess<'de>
	{
		
		let is_channel: u8 = match seq.next_element()? {
			None => return Err( de::Error::custom("no bytes") ),
			Some(x) => x
		};

		let result = match is_channel {
			0 => EventType::Channel,
			1 => {
				let address = match seq.next_element()? {
					None => Err( de::Error::custom("not enough bytes to extract the address") )?,
					Some(x) => x
				};

				EventType::Publisher( address )
			},
			other => Err( de::Error::custom("unknown event type found") )?
		};

		Ok( result )
	}
}