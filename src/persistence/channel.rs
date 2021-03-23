use std::{
	ops::Deref,
};

use async_std::{
	prelude::*,
};
use bincode;
use gnunet::{
	crypto::*,
	identity::*
};
use rusqlite::*;

use crate::{
	persistence::{
		self,
		Result
	},
	message::*
};



#[derive(Clone)]
pub struct Handle {
	pub base: persistence::Handle,
	pub id: i64
}



impl Handle {
	
	pub async fn load_address( &self ) -> Result<PublicKey> {

		let key = self.base.query("SELECT address FROM channel WHERE id = ?",
			params![self.id],
			|_, mut rows| {
				let address: String = rows.next()?.expect("row not found").get(0)?;
				Ok( PublicKey::from_string( &address ) )
			}
		).await?;

		Ok( key.expect("address incorrectly formatted") )
	}

	/// Stores the profile for this channel.
	pub async fn store_profile( &self, profile: &ChannelProfile ) -> Result<()> {
		let data = bincode::serialize( profile )?;

		let profile_picture = profile.base.profile_picture.as_ref().map(|h| h.to_string());
		let stylesheet = profile.stylesheet.as_ref().map(|h| h.to_string());

		let result: Option<i64> = self.base.query_one("SELECT profile_id FROM channel_profile WHERE channel_id = ?",
			params![self.id],
			|_, row| Ok( row.get(0)? )
		).await?;

		if let Some(profile_id) = result {

			self.base.execute_one("UPDATE profile SET title = ?, description = ?, picture_hash = ? WHERE id = ?",
				params![
					profile.base.title,
					profile.base.description,
					profile_picture,
					profile_id
				]
			).await?;

			self.base.execute_one("UPDATE channel_profile SET stylesheet = ? WHERE ROWID = ?",
				params![
					stylesheet
				]
			).await?;
		}
		else {
			let profile_id = self.base.insert("INSERT INTO profile (title, description, picture_hash) VALUES (?,?,?); RETURNING ROWID",
				params![
					&profile.base.title,
					&profile.base.description,
					profile_picture,
					self.id
				]
			).await?;

			self.base.insert("INSERT INTO channel_profile (channel_id, profile_id, stylesheet) VALUES (?,?,?)",
				params![
					self.id,
					profile_id,
					stylesheet
				]
			).await?;
		}

		Ok(())
	}

	pub async fn fetch_profile( &self ) -> Result<Option<ChannelProfile>> {

		Ok( self.base.query_one("SELECT p.revision, p.title, p.description, p.picture_hash, c.stylesheet FROM channel_profile INNER JOIN channel_profile ON c.channel_id = p.ROWID WHERE o.ROWID = ?",
			params![self.id],
			|_, row| {
				let hash_string: Option<String> = row.get(2)?;
				let hash = hash_string.map(|s| HashCode::from_string(&s).expect("invalid hash code"));
				let stylesheet_hash_string: Option<String> = row.get(3)?;
				let stylesheet_hash = stylesheet_hash_string.map(|s| HashCode::from_string(&s).expect("invalid hash code"));

				let revision: i64 = row.get(0)?;

				Ok( ChannelProfile {
					base: Profile {
						revision: revision as _,
						title: row.get(1)?,
						description: row.get(2)?,
						profile_picture: hash
					},
					stylesheet: stylesheet_hash
				} )
			}
		).await? )
	}

	/// Stores an event message with the given id.
	/// Storing multiple messages with the same id is possible.
	pub async fn store_event( &self, id: u64, message: &[u8] ) -> Result<()> {

		self.base.insert("INSERT INTO channel_event (id, channel_id, message) VALUES (?,?,?)",
			params![id as i64, self.id, message]).await?;
		Ok(())
	}
}

/// Devides the given `data` up into blocks of `BLOCK_LENGTH` length.
/// The last block may be smaller.
fn breakup_data<'a>( data: &'a [u8], block_len: usize ) -> Vec<&'a [u8]> {
	let mut i = 0;

	// Calculate block count
	let mut block_count = data.len() / block_len;
	if data.len() % block_len > 0 {
		block_count += 1;
	}

	let mut blocks = Vec::with_capacity( block_count );

	// Device
	loop {

		if (data.len() - i) > block_len {
			let end = i + block_len;
			blocks.push( &data[ i..end ] );
		} else {
			blocks.push( &data[ i.. ] );
			break;
		}

		i += block_len;
	}

	blocks
}

fn hash_blocks( blocks: &[&[u8]] ) -> Vec<HashCode> {

	let mut results = Vec::with_capacity( blocks.len() );

	for block in blocks {

		let hash = gnunet::crypto::HashCode::generate( block );
		results.push( hash.into() );
	}

	results
}

impl Deref for Handle {
	type Target = persistence::Handle;

	fn deref( &self ) -> &Self::Target {
		&self.base
	}
}