use std::io;

use gnunet::{
	crypto::HashCode,
	identity::PublicKey
};



pub trait Signature {
	fn verify_hash( &self, hash: &HashCode, public_key: &PublicKey ) -> bool;
}



impl Signature for gnunet::identity::Signature {
	fn verify_hash( &self, hash: &HashCode, public_key: &PublicKey ) -> bool {
		self.verify( 777, &hash.to_bytes(), public_key )
	}
}



pub fn into_io_data_error<E>( error: E ) -> io::Error where
	E: Into<Box<dyn std::error::Error + Send + Sync>>
{
	io::Error::new( io::ErrorKind::InvalidData, error )
}