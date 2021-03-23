use gnunet::{
	crypto::HashCode,
	identity::Signature
};
use serde::{Serialize, Deserialize};



#[derive(Clone, Deserialize, Serialize)]
pub struct Attachment {
	pub block_ids: Vec<HashCode>
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PostInfo {
	pub publish_timestamp: u64,
	pub tags: Vec<String>
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Post {
	pub id: u64,
	/// The hash of the post `data`,
	pub hash: HashCode,
	pub signature: Signature,
	pub meta: PostMeta
}

#[derive(Clone, Deserialize, Serialize)]
pub struct PostMeta {
	/// Some extra information about the post
	pub info: PostInfo,
	/// The id of the post that foregoes this one.
	/// This is useful for obtaining older posts, as there is no 'index' for all posts of a blog.
	pub content_hash: HashCode,
	/// The ids of the files that this post holds as attachments.
	/// E.g. photos, sound bites, video's, or basically anything.
	pub attachment_ids: Vec<HashCode>
}