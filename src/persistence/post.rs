use std::{
	fmt,
};

use async_std::{
	prelude::*
};
use gnunet::{
	crypto::*,
};
use rusqlite::params;
use thiserror::Error;

use crate::{
	persistence::{
		timeline,
		Result
	}
};



/// The block length used for attachments and other files.
pub const FILE_BLOCK_LENGTH: usize = 1024*1024;



#[derive(Clone)]
pub struct Handle {
	pub timeline: timeline::Handle,
	pub id: i64
}

#[derive(Debug, Error)]
pub enum PostError {
	InvalidBlockSize( HashCode, usize )
}



impl Handle {

	pub async fn load_block( &self, block_id: &HashCode ) -> Result<Option<Vec<u8>>> {
		
		Ok( self.timeline.base.query_one("SELECT data FROM block WHERE hash = ?",
			params![block_id.to_string()],
			|_, row| row.get(0)
		).await? )
	}

	pub async fn load_content( &self ) -> Result<Option<String>> {
		
		Ok( self.timeline.base.query_one("SELECT data FROM post_content WHERE ROWID = (SELECT content_id FROM post WHERE ROWID = ?)",
			params![self.id],
			|_, row| row.get(0)
		).await? )
	}

	/*/// Retrieves the reStructuredText content of the post to the best of our ability.
	/// Returns the reStructuredText a vector of strings.
	/// The meaning of this is that if one string is returned, the whole content has been loaded succesfully.
	/// If two or more strings are returned, it means that blocks in between were missing.
	/// If the last string is empty, it means that the last block was missing.
	pub async fn load_content( &self, post_id: u64 ) -> io::Result<Vec<String>> {
		// TODO: Return a vector of strings that represents all the content that is available at the moment.
		//       So basically, when one block is missing, this would return a vector of two strings, representing the parts of the content that could still be found.
		let buffer_capacity = block_ids.len() * POST_BLOCK_LENGTH;

		let mut results = Vec::new();
		
		let mut buffer = vec![0u8; buffer_capacity ];
		let mut last_block_len: usize = 0;
		let mut start_block = 0;

		fn push_result( results: &mut Vec<String>, i: usize, start_block: &mut usize, buffer: &[u8] ) {
			let begin = i * POST_BLOCK_LENGTH;

			if i != *start_block {
				let start = *start_block * POST_BLOCK_LENGTH;
				let part = String::from_utf8_lossy( &buffer[start..begin] );
				results.push( part.to_string() );
				*start_block = i+1;
			}
		}

		// For each block, create a future which fills the content vector with its block data.
		for i in 0..block_ids.len() {
			let block_id = block_ids[i].clone();

			match File::open( self.path.join("blocks").join(block_id.to_string()) ).await {
				Err(e) => {
					if e.kind() == io::ErrorKind::NotFound {
						push_result( &mut results, i, &mut start_block, &*buffer );
					}
					else {
						return Err(e);
					}
				},
				Ok(mut file) => {
					let begin = i * POST_BLOCK_LENGTH;
					let end = begin + POST_BLOCK_LENGTH;

					let read = file.read( &mut buffer[begin..end] ).await?;
	
					// Only the last block is supposed to be smaller than POST_BLOCK_LENGTH
					if read < POST_BLOCK_LENGTH {
						if i == (block_ids.len()-1) {
							push_result( &mut results, i, &mut start_block, &*buffer );
							return Err( io::Error::new( io::ErrorKind::InvalidData, PostError::InvalidBlockSize( block_id.clone(), read ) ) );
						}
						else {
							last_block_len = read;
						}
					}
				}
			}
		}

		let start = start_block * POST_BLOCK_LENGTH;
		let end = (buffer.len() - 1) * POST_BLOCK_LENGTH + last_block_len;
		let part = String::from_utf8_lossy( &buffer[start..end] );
		results.push( part.to_string() );

		Ok( results )
	}*/

	pub async fn store_block( &self, id: &HashCode, block: &[u8] ) -> Result<()> {

		self.timeline.base.insert("INSERT INTO block (hash, data) VALUES (?,?)",
			params![id.to_string(), block]
		).await?;

		Ok(())
	}

	pub async fn store_blocks( &self, ids: &[HashCode], blocks: &[&[u8]] ) -> Result<()> {

		for i in 0..ids.len() {
			let id = &ids[i];
			let block = blocks[i];
	
			self.store_block( id, block ).await?;
		}

		Ok(())
	}

	/// Stores the content `data` for the post with the given `id`.
	pub async fn store_content( &self, body: &str ) -> Result<i64> {

		let content_id = self.timeline.base.insert("INSERT INTO post_content (body) VALUES (?)", params![body]).await?;

		self.timeline.base.execute_one("UPDATE post SET content_id = ? WHERE ROWID = ?", params![content_id as i64, self.id]).await?;

		Ok(content_id)
	}
}

impl fmt::Display for PostError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidBlockSize(hash, size) => write!(f, "block {} has an invalid block size of {} bytes", &hash.to_string(), size)
		}
	}
}