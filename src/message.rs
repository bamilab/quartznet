//! This module contains all network messages.
//! So requests and responses, but also notifications.

use std::{
	collections::HashMap,
	fmt
};

use gnunet::{
	crypto::HashCode,
	identity::Signature
};
use serde::{*, ser::SerializeTuple};

use crate::{
	byte_enum,
	post::*
};



pub const PROFILE_DESCRIPTION_MAX_LEN: u16 = 1024;

byte_enum! {
	pub enum MessageDirectionType {
		/// An event that needs to be redistributed.
		Event = 0,
		Request = 1,
		Response = 2
	}
}

byte_enum! {
	pub enum RequestType {
		/// Requests the meta data or content of a post.
		Posts = 0,
		/// Request a number of files
		Files,
		/// Request blocks of a file
		Blocks
	}
}

byte_enum! {
	pub enum ResponseResultType {
		Success = 0,
		InternalError
	}
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ProtocolVersion {
	major: u16,
	minor: u16
}

/// Requests the data of a block of a post.
#[derive(Clone, Deserialize, Serialize)]
pub struct BlocksRequest {
	pub post_id: HashCode,
	pub block_ids: Vec<HashCode>
}

/// The request send to one of the relays for the channel,
///  requesting the last message that was published.
#[derive(Clone, Deserialize, Serialize)]
pub struct ChannelLastMessageRequest {}

#[derive(Clone, Deserialize, Serialize)]
pub struct ChannelLastMessageResponse {
	message_id: HashCode
}

/// A response to `BlockRequest`.
/// This contains the data of the blocks that were requested.
#[derive(Clone, Deserialize, Serialize)]
pub struct BlocksResponse {
	pub data: Vec<Vec<u8>>
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Message {
	id: HashCode,
	previous_message_id: HashCode,
	type_: u8,
	
}

/// Requests the meta info of a post
#[derive(Clone, Deserialize, Serialize)]
pub struct PostMetaRequest {
	pub post_ids: Vec<HashCode>
}

/// Response to `PostMetaRequest`.
/// The response may or may not contain all post ids that the request requested.
#[derive(Clone, Deserialize, Serialize)]
pub struct PostMetaResponse {
	pub metas: HashMap<HashCode, PostMeta>
}

/// A message that is sent to all subscribers of a blog, to indicate there is a new post to available.
#[derive(Clone, Deserialize, Serialize)]
pub struct PostNotification {
	pub post_id: Signature
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PostSearchRequest {
	pub keywords: Vec<String>
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PostSearchResponse {
	pub posts: Vec<Post>
}

#[derive(Clone)]
pub struct Profile {
	pub revision: u64,
	pub title: String,
	pub description: String,
	pub profile_picture: Option<HashCode>
}
struct ProfileVisitor;

#[derive(Clone, Deserialize, Serialize)]
pub struct ChannelProfile {
	pub base: Profile,
	pub stylesheet: Option<HashCode>
}

/// The message to notify the channel swarm of new profile information.
#[derive(Clone, Deserialize, Serialize)]
pub struct UpdateChannelProfileEventMessage {
	pub hash: HashCode,
	pub signature: Signature,
	pub profile: ChannelProfile
}



impl Serialize for Profile {

	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where
		S: Serializer
	{
		let len = 2 + self.title.len() + 1 + self.description.len() + 2;
		let mut s = serializer.serialize_tuple(len)?;

		s.serialize_element( &self.revision )?;

		// Title
		let len = self.title.len() as u8;
		s.serialize_element( &len )?;
		let buf = self.title.as_bytes();
		for i in 0..self.title.len() {
			s.serialize_element( &buf[i] )?;
		}

		// Description
		let len = self.description.len() as u16;
		s.serialize_element( &len )?;
		let buf = self.description.as_bytes();
		for i in 0..self.description.len() {
			s.serialize_element( &buf[i] )?;
		}

		// Profile picture
		s.serialize_element( &self.profile_picture )?;

		s.end()
	}
}

impl<'de> Deserialize<'de> for Profile {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where
		D: Deserializer<'de>
	{
		deserializer.deserialize_tuple( 0, ProfileVisitor )
	}
}

impl fmt::Display for Profile {
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result {
		write!(f, "{}", self.to_string())
	}
}

impl<'de> de::Visitor<'de> for ProfileVisitor {

	type Value = Profile;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a profile structure")
	}

	fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
		A: de::SeqAccess<'de>
	{
		
		let revision: u64 = match seq.next_element()? {
			None => return Err( de::Error::custom("not enough bytes to extract the revision") ),
			Some(x) => x
		};

		// Title
		let len: u8 = seq.next_element()?.unwrap();
		let mut buf = vec![0u8; len as usize];
		for i in 0..buf.len() {
			buf[i] = seq.next_element()?.unwrap();
		}
		let title = match String::from_utf8( buf ) {
			Err(_) => return Err( de::Error::custom("not enough bytes to extract the title") ),
			Ok(x) => x
		};

		// Description
		let len: u16 = seq.next_element()?.unwrap();
		let mut buf = vec![0u8; len as usize];
		for i in 0..buf.len() {
			buf[i] = seq.next_element()?.unwrap();
		}
		let description = match String::from_utf8( buf ) {
			Err(_) => return Err( de::Error::custom("not enough bytes to extract the description") ),
			Ok(x) => x
		};

		let profile_picture: Option<HashCode> = match seq.next_element()? {
			None => return Err( de::Error::custom("not enough bytes to extract the profile picture hash") ),
			Some(x) => x
		};

		Ok( Profile {
			revision,
			title,
			description,
			profile_picture
		} )
	}
}