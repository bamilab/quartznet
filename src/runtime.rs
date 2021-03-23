use std::future::Future;
/*use std::mem;
use std::panic;
use std::thread;*/

use tokio;



pub fn spawn( future: impl Future<Output=()> + Send + 'static ) {
	tokio::spawn( future );
}

/// Blocks the current thread in a way that doesn't intervene with the runtime.
pub async fn block_on<F, R>( func: F ) -> R where
	F: FnOnce() -> R,
{
	// We wrap the closure in a closure that catches any panic, and returns a result that is Err(...) if it panicked.
	// This way, we can panic on the calling task.
	//let wrapper = move || {
	//	panic::catch_unwind( func )
	//};

	// This is a lifetime hack.
	// The closure isn't really 'static, but it is a requirement for tokio's spawn_blocking function.
	// And because we know our function ends after the closure ends, this should still be safe.
	//let boxed = Box::new(wrapper) as Box<dyn FnOnce() -> thread::Result<R> + Send>;
	//let unsafe_box: Box<dyn FnOnce() -> thread::Result<R> + Send + 'static> = unsafe { mem::transmute(boxed) };

	//tokio::task::spawn_blocking( unsafe_box ).await.map_err(|e| eprintln!("error while blocking: {}", e)).expect("blocking error").expect("panic in blocking closure")
	tokio::task::block_in_place( func )
}