use std::{
	fmt,
	ops::{Deref, DerefMut},
	panic::{UnwindSafe, AssertUnwindSafe},
	path::*,
	sync::{Arc, Mutex}
};

use async_std::{
	prelude::*
};
use fallible_iterator::FallibleIterator;
use gnunet::{
	identity::{self, *}
};
use lazy_static::lazy_static;
use rusqlite::{self, NO_PARAMS, params, types::ToSql};
use unsafe_send_sync::*;

use crate::runtime;

pub mod channel;
pub mod post;
pub mod timeline;



lazy_static! {
	pub static ref DATABASE_DIR: PathBuf = PathBuf::from( "/home/bamilab/.quartznet" );
}

pub struct Connection ( rusqlite::Connection );

#[derive(Clone)]
pub struct Handle {
	gnunet: gnunet::Handle,
	db: Arc<Mutex<Connection>>
}

/// The error that may occur during the creation of a particular database model.
#[derive(Debug)]
pub enum Error {
	/// The given information already exists for another entry of the model.
	AlreadyExists,
	/// Gnunet error
	Gnunet( gnunet::Error ),
	/// A SQL error
	Database( rusqlite::Error ),
	// Any errors serializing structures into bytes or the other way around.
	Serialization( bincode::Error )
}

pub type Result<T> = std::result::Result<T, Error>;



impl Connection {
	pub fn query<P, F, R>( &self, sql: &'static str, params: P, on_result: F ) -> rusqlite::Result<R> where
		P: IntoIterator,
		P::Item: ToSql,
		F: FnOnce(rusqlite::Rows) -> rusqlite::Result<R>
	{
		let mut statement = self.0.prepare( sql )?;
		let result = statement.query(params)?;

		on_result(result)
	}
}

impl Handle {

	/// Creates a new ego and saves it to
	pub async fn create_channel( &mut self, name: &str ) -> Result<channel::Handle> {

		let mut identity_service = identity::Handle::connect( self.gnunet.clone() ).await?;

		let private_key = PrivateKey::generate( KeyType::Eddsa );
		let success = identity_service.create( name, private_key.clone() ).await?;
		if !success { return Err( Error::AlreadyExists ) }

		let public_key = Arc::new( private_key.extract_public().unwrap() );
		let address_str = public_key.to_string();
		
		let row_id = self.query("INSERT INTO channel (address) VALUES (?); RETURNING ROWID", params![address_str], |_, mut rows| {
			rows.next()?.unwrap().get(0)
		}).await?;

		self.own_channel( name, &public_key ).await?;

		Ok(channel::Handle {
			base: self.clone(),
			id: row_id
		})
	}

	pub async fn execute<P, F, R>( &self, sql: &'static str, params: P, on_executed: F ) -> rusqlite::Result<R> where
		P: IntoIterator,
		P::Item: ToSql,
		F: FnOnce(u64) -> rusqlite::Result<R>
	{
		let db = self.db.clone();
		// params is safe to send to another thread because it does not get used anymore after this function, and it is not copied or cloned.
		let params = UnsafeSend::new( params );
		// The same goes for `on_executed`
		let on_executed = UnsafeSend::new( on_executed );

		runtime::block_on(move || {
			let guard = db.lock().unwrap();
			let mut statement = guard.prepare( sql )?;
			let affected = statement.execute(params.unwrap())?;

			(on_executed.unwrap())(affected as _)
		}).await
	}

	pub async fn execute_expect<P>( &self, sql: &'static str, params: P, expected_rows: u64 ) -> rusqlite::Result<()> where
		P: IntoIterator,
		P::Item: ToSql
	{
		self.execute(sql, params, |affected| Ok( debug_assert!(affected != expected_rows, "unexpected number of rows affected: {} (should be {})", affected, expected_rows) ) ).await
	}

	pub async fn execute_one<P>( &self, sql: &'static str, params: P ) -> rusqlite::Result<()> where
		P: IntoIterator,
		P::Item: ToSql
	{
		self.execute_expect( sql, params, 1 ).await
	}

	pub async fn insert<P>( &self, sql: &'static str, params: P ) -> rusqlite::Result<i64> where
		P: IntoIterator,
		P::Item: ToSql
	{
		let db = self.db.clone();

		runtime::block_on(move || {
			let guard = db.lock().unwrap();
			let mut statement = guard.prepare( sql )?;
			statement.insert(params)
		}).await
	}

	pub async fn query<P, F, R>( &self, sql: &'static str, params: P, on_result: F ) -> rusqlite::Result<R> where
		P: IntoIterator,
		P::Item: ToSql,
		F: FnOnce(&Connection, rusqlite::Rows) -> rusqlite::Result<R>
	{
		let db = self.db.clone();
		// params is safe to send to another thread because it does not get used anymore after this function, and it is not copied or cloned.
		// params is also UnwindSafe, because it is simply just references of references. No mutability exists.
		// the rusqlite::params macro generates something like `&[&dyn ToSql]`, but we don't have the power to change this type to `&[&dyn ToSql + RefUnwindSafe]`.
		//let params = UnsafeSend::new( params );
		// The same goes for `on_result`
		//let on_result = UnsafeSend::new( on_result );

		runtime::block_on(move || {
			let guard = db.lock().unwrap();
			
			guard.query( sql, params, |rows| {
				on_result( &*guard, rows )
			})
		}).await
	}

	/// Queries one row.
	pub async fn query_one<P, F, R>( &self, sql: &'static str, params: P, on_result: F ) -> rusqlite::Result<Option<R>> where
		P: IntoIterator,
		P::Item: ToSql,
		F: FnOnce(&Connection, &rusqlite::Row<'_>) -> rusqlite::Result<R>
	{
		self.query( sql, params, |con, mut rows| {
			let result = match rows.next()? {
				None => None,
				Some(row) => {
					Some( on_result(con, row)? )
				}
			};
			Ok( result )
		}).await
	}

	pub async fn list_channels( &self ) -> Result<Vec<channel::Handle>> {
		
		let b = self.clone();
		let channels: Vec<channel::Handle> = self.query("SELECT ROWID FROM channel", NO_PARAMS,
			move |_, rows| Ok(rows.map(|row| Ok( channel::Handle {
				base: b.clone(),
				id: row.get(0)?
			}) ).collect()?)
		).await?;

		Ok( channels )
	}

	/// Retrieves all names of all ego's that have a blog.
	pub async fn list_my_timelines( &self ) -> Result<Vec<timeline::Handle>> {
		
		let b = self.clone();
		let timelines = self.query("SELECT ROWID FROM publisher WHERE ROWID IN (SELECT publisher_id FROM local_publishers)", NO_PARAMS,
			move |_, rs| Ok( rs.map(|r| Ok( timeline::Handle {
				base: b.clone(),
				id: r.get(0)?
			})).collect()? ) ).await?;

		Ok( timelines )
	}

	pub async fn get_latest_id( &self, id_type: &str ) -> Result<Option<u64>> {

		let result: Option<i64> = self.query_one("SELECT id FROM latest_ids WHERE type = ?",
			params![id_type],
			|_, row| row.get(0)
		).await?;

		Ok( result.map(|i| i as _) )
	}

	/// Marks the ego identified with the given name, as an ego that belongs .
	pub async fn own_channel( &self, name: &str, address: &PublicKey ) -> Result<bool> {

		let _ = self.insert("INSERT INTO local_publishers (publisher_id) VALUES ((SELECT ROWID FROM publishers WHERE address = ?))",
			params![address.to_string()]
		).await?;

		Ok(true)
	}

	pub async fn connect( gnunet: gnunet::Handle ) -> rusqlite::Result<Self> {
		 
		let db_conn = runtime::block_on(|| {
			rusqlite::Connection::open(DATABASE_DIR.join("db.sqlite"))
		}).await?;

		Ok(Self {
			gnunet,
			db: Arc::new( Mutex::new( Connection ( db_conn ) ) )
		})
	}

	pub async fn get_channel( self, id: &PublicKey ) -> Result<Option<channel::Handle>> {

		let channel = self.query_one("SELECT ROWID FROM channel WHERE address = ?", params![id.to_string()],
			|_, row| { Ok( channel::Handle {
				base: self.clone(),
				id: row.get(0)?
			} )
		}
		).await?;
		
		Ok( channel )
	}

	pub async fn get_timeline( &self, publisher_address: &PublicKey ) -> Result<Option<timeline::Handle>> {

		let timeline = self.query_one("SELECT ROWID FROM publisher WHERE address = ?", params![publisher_address.to_string()],
			|_, row| { Ok( timeline::Handle {
					base: self.clone(),
					id: row.get(0)?
				} )
			}
		).await?;
	
		Ok( timeline )
	}
}



impl Deref for Connection {
	type Target = rusqlite::Connection;

	fn deref( &self ) -> &Self::Target {
		&self.0
	}
}

impl DerefMut for Connection {
	fn deref_mut( &mut self ) -> &mut Self::Target {
		&mut self.0
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::AlreadyExists => write!(f, "already exists"),
			Self::Gnunet(e) => write!(f, "gnunet error: {}", e),
			Self::Database(e) => write!(f, "database error: {}", e),
			Self::Serialization(e) => write!(f, "(de)serialization error: {}", e)
		}
	}
}


impl From<gnunet::Error> for Error {
	fn from( e: gnunet::Error ) -> Self {
		Self::Gnunet(e)
	}
}
impl From<rusqlite::Error> for Error {
	fn from( e: rusqlite::Error ) -> Self {
		Self::Database(e)
	}
}
impl From<bincode::Error> for Error {
	fn from( e: bincode::Error ) -> Self {
		Self::Serialization(e)
	}
}