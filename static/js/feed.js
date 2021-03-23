function set_error_status( post ) {

	var el = document.getElementById("feed-status")

	el.innerText = "Error: " + post.message
	el.className += " error"
}

function set_status( message ) {
	var el = document.getElementById("feed-status")
	el.innerText = message
}

function add_blog_post( post ) {
	
	var el = document.createElement("div")
	el.className = "feed-post"
	el.innerText = post.html

	document.getElementById("posts").prepend( el )
}

async function load_posts( address, post_handler ) {
	let socket = new WebSocket("/api/posts/" + address)

	return new Promise((ok, err) => {

		socket.onopen = () => console.log(""),
		socket.onerror = (e) => err(e),
		socket.onmessage = (e) => {
			let object = JSON.parse( e.data )

			post_handler( object )
		}
	})
}



await load_posts( ADDRESS, (post) => {

	if ( 'error' in post ) {
		set_error_status( post )
	}
	else {
		append_post( post )
	}
})
