//! This module provides the persistence functionality of the timeline.
//! 
//! A timeline is the history of posts that a publisher maintains.
//! The timeline consists of the following events:
//! * Publishing of a post
//! * Revision of the content of a post
//! * Request to forget a post (a.k.a. post deletion)

use std::{
	convert::TryInto,
	str,
	time::SystemTime
};

use async_std::{
	prelude::*
};
use bincode;
use fallible_iterator::FallibleIterator;
use gnunet::{
	crypto::*,
	identity::*
};
use rusqlite::params;

use crate::{
	persistence::{
		self,
		post,
		Result
	},
	post::*
};



#[derive(Clone)]
pub struct Handle {
	pub base: persistence::Handle,
	pub id: i64
}



/// The block length used for 
pub const POST_BLOCK_LENGTH: usize = 1024;
/// The purpose used for the signatures
pub const POST_SIGNATURE_PURPOSE: u32 = 777;

impl Handle {

	pub async fn create_post( &self, private_key: &PrivateKey, content: &str, info: PostInfo ) -> Result<(post::Handle, Post)> {

		let post_id = match self.load_latest_post_id().await? {
			None => 0,
			Some(latest_id) => latest_id + 1
		};
		let content_hash = HashCode::generate( content.as_bytes() );

		let tags = info.tags.clone();

		let post_data = PostMeta {
			info,
			content_hash,
			attachment_ids: Vec::new()
		};
		let raw_post_data = bincode::serialize( &post_data ).expect("unable to serialize post data");
		let post_hash = HashCode::generate( &*raw_post_data );

		let raw_post_hash = bincode::serialize( &post_hash ).expect("unable to serialize post ID");
		let signature = private_key.sign( (&*raw_post_hash).try_into().unwrap(), POST_SIGNATURE_PURPOSE ).unwrap();
		let raw_signature = bincode::serialize( &signature ).expect("unable to serialize signature");

		let post_handle = self.clone().into_post( post_id as _ );
		let content_id = post_handle.store_content( content ).await?;

		let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

		let row_id = self.base.insert("INSERT INTO post (id, publisher_id, hash, signature, publish_timestamp, content_hash, attachment_count, content_id VALUES (?,?,?,?,?,?,?)",
			params![
				post_id as i64,
				self.id,
				post_hash.to_string(),
				bincode::serialize(&signature)?,
				timestamp.as_millis() as i64,
				post_data.content_hash.to_string(),
				0i64,
				content_id
			]
		).await?;

		self.index_tags( row_id, &*tags ).await?;

		let handle = post::Handle {
			timeline: self.clone(),
			id: row_id as _
		};
		Ok((handle, Post {
			id: post_id,
			hash: post_hash,
			signature,
			meta: post_data
		}))
	}

	pub async fn get_my_ego( &self ) -> Result<Option<String>> {

		Ok( self.base.query_one("SELECT ego FROM local_publishers WHERE publisher_id = ?",
			params![self.id],
			|_, row| row.get(0)
		).await? )
	}

	/// Loads the post if it is available locally.
	/// If the post is not available locally, return `None`.
	pub async fn load_post( &self, post_id: u64 ) -> Result<Option<Post>> {

		let post = self.base.query_one("SELECT publisher_id, hash, signature, publish_timestamp, content_id, attachment_count FROM post WHERE id = ?",
			params![post_id as i64],
			|con, row| {
				let attachment_count: i64 = row.get(5)?;
				let hash_str: String = row.get(1)?;
				let signature: Vec<u8> = row.get(2)?;
				let timestamp: i64 = row.get(3)?;
				let content_id: String = row.get(4)?;
				
				let tags: Vec<String> = con.query("SELECT keyword FROM tags WHERE post_id = (SELECT ROWID FROM post WHERE id = ?)",
					params![post_id as i64],
					|rows| Ok( rows.map(|row| row.get(0)).collect()? )
				)?;

				Ok( Post {
					id: post_id,
					hash: HashCode::from_string( &hash_str ).unwrap(),
					signature: bincode::deserialize( &*signature ).unwrap(),
					meta: PostMeta {
						info: PostInfo {
							publish_timestamp: timestamp as _,
							tags
						},
						content_hash: HashCode::from_string( &content_id ).unwrap(),
						attachment_ids: Vec::new()
					}
				})
			}
		).await?;

		Ok( post )
	}

	async fn index_tags( &self, post_row_id: i64, tags: &[String] ) -> Result<()> {
		
		for keyword in tags {
			self.base.insert("INSERT INTO tags (keyword, post_id) VALUES (?,?)",
				params![keyword, post_row_id]).await?;

		}

		Ok(())
	}
	
	pub async fn list_posts( &mut self, start: u64, count: u16 ) -> Result<Vec<Option<Post>>> {
		debug_assert!(count > 0, "count should be positive");

		let latest_post_id = match self.load_latest_post_id().await? {
			None => return Ok( Vec::new() ),
			Some(x) => x
		};

		// Accumatively add all posts
		let mut posts = Vec::with_capacity( count as usize );
		let end = start + count as u64;
		for id in start..end {

			let post = self.load_post( id ).await?;
			posts.push(post);
		}

		Ok( posts )
	}

	pub fn into_post( self, post_row_id: i64 ) -> post::Handle {

		post::Handle {
			timeline: self,
			id: post_row_id
		}
	}

	async fn load_latest_post_id( &self ) -> Result<Option<u64>> {
		
		let id: Option<i64> = self.base.query_one("SELECT last_post_id FROM publisher WHERE ROWID = ?",
			params![self.id],
			|_, row| row.get(0)
		).await?;

		Ok( id.map(|i| i as _) )
	}

	/// Stores an event message with the given id.
	/// Storing multiple messages with the same id is possible.
	pub async fn store_event( &self, id: u64, message: &[u8] ) -> Result<()> {

		self.base.insert("INSERT INTO publisher_event (id, publisher_id, message) VALUES (?,?,?)",
			params![id as i64, self.id, message]).await?;
		Ok(())
	}

	async fn update_latest_post_id( &self, post_id: u64 ) -> Result<()> {

		self.base.execute_one("UPDATE publisher SET last_post_id = ? WHERE ROWID = ?",
			params![post_id as i64, self.id]
		).await?;

		Ok(())
	}
}