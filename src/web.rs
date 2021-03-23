use actix_web::{error, get, http::header, HttpResponse, HttpRequest, post, web};
use gnunet::{
	self,
	crypto::HashCode,
	identity::*
};
use serde::*;
use rusqlite;
use tera;

use std::{
	cmp::min,
	collections::HashMap,
	io,
	sync::Arc,
	time::{SystemTime, UNIX_EPOCH}
};

use crate::persistence::{self, timeline};
use crate::Globals;
use crate::post::*;



#[derive(Serialize)]
pub struct Blog {
	name: String,
	address: String
}

#[derive(Serialize)]
pub struct HomepageContext {
	follower_count: u32,
	following_count: u32,

	following: Vec<Blog>,
	own_blogs: Vec<String>
}

#[get("/")]
pub async fn homepage(g: web::Data<Arc<Globals>>, _req: HttpRequest) -> error::Result<HttpResponse> {

	#[derive(Serialize)]
	struct Blog {
		name: String,
		address: String
	}

	let p = persistence::Handle::connect( g.gnunet.clone() ).await.map_err(|e| persistence::Error::Database(e))?;
	let my_timelines = p.list_my_timelines().await?;
	let mut blogs = Vec::with_capacity( my_timelines.len() );

	let mut identity_service = gnunet::identity::Handle::connect( g.gnunet.clone() ).await
		.map_err(|_| error::ErrorInternalServerError("Gnunet identity sere.into::<persistence::Error>()vice not available."))?;

	for timeline in my_timelines {
		let name = timeline.get_my_ego().await?.unwrap();
		let priv_key = identity_service.lookup( &name ).await
			.map_err(|e| { eprintln!("Unable to find ego with name \"{}\" due to error: {}", &name, e); error::ErrorInternalServerError("Internal server error") })?
			.ok_or_else(|| { eprintln!("Unable to find ego with name \"{}\".", &name); error::ErrorInternalServerError("Internal server error") })?;
		let pub_key = priv_key.extract_public().unwrap();

		blogs.push( Blog {
			name,
			address: pub_key.to_string()
		} )
	}

	let mut context = tera::Context::new();
	context.insert("own_blogs", &blogs);

	let html = g.tera.render("homepage.html", &context)
		.map_err(|e| { eprintln!("Template error: {}", e); error::ErrorInternalServerError("Template error") } )?;
	Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

#[get("/channel/new")]
pub async fn channel_new(g: web::Data<Arc<Globals>>) -> error::Result<HttpResponse> {

	let html = g.tera.render("blog-new.html", &tera::Context::new())
		.map_err(|e| { eprintln!("Template error: {}", e); error::ErrorInternalServerError("Template error") } )?;

	Ok(HttpResponse::Ok().content_type("text/html").body(html))
}



#[derive(Deserialize)]
pub struct FormData {
	name: String
}

#[post("/channel/new")]
pub async fn channel_new_post<'s>(g: web::Data<Arc<Globals>>, form: web::Form<FormData>) -> error::Result<HttpResponse> {
	
	let mut db = persistence::Handle::connect( g.gnunet.clone() ).await.map_err(|e| persistence::Error::Database(e))?;

	let result = match db.create_channel( &form.name ).await {
		Err(e) => {
			match e {
				persistence::Error::AlreadyExists => {
					Err( error::ErrorBadRequest( "An ego with that name already exists!" ) )
				},
				err => {
					eprintln!("Internal server error: {}", err);
					Err( error::ErrorConflict( "Internal server error occurred." ) )
				}
			}
		},
		Ok(_) => {
			Ok( HttpResponse::Found().append_header((header::LOCATION, "/")).finish() )
		}
	};

	result
}

#[derive(Deserialize)]
pub struct BlogFeedParams {
	id: String,
	id_type: String,
	page: u32
}

#[derive(Deserialize)]
pub struct BlogFeedIdParams {
	id: String,
	id_type: String
}

#[derive(Serialize)]
pub struct PostPreview {
	id: String,
	info: PostInfo,
	html: String
}

async fn load_post_previews( blog_: &timeline::Handle, posts: &[Option<Post>] ) -> error::Result<Vec<PostPreview>> {
	let mut previews = Vec::with_capacity( posts.len() );
	
	for opt_post in posts {

		if let Some(post) = opt_post {
			let blog = blog_.clone();
			let persistence = blog.into_post( post.id.clone() as _ );	// TODO: Load post by postid

			let content = persistence.load_content().await?.expect("missing content");
			let end = min( 257, content.len() );
			let mut preview = String::from_utf8_lossy( &content.as_bytes()[..end] ).to_string();
			preview.pop();	// The last character may not have been valid UTF-8 because perhaps not all bytes of the character where contained within this block.
			// This is a quickfix for dealing with that.

			previews.push(PostPreview {
				id: post.id.to_string(),
				info: post.meta.info.clone(),
				html: preview
			})
		}
	}

	Ok( previews )
}

#[get("/channel/feed/{id_type}/{id}/{page}")]
pub async fn channel_feed(g: web::Data<Arc<Globals>>, p: web::Path<BlogFeedParams>) -> error::Result<HttpResponse> {
	_channel_feed(g, &p.id, &p.id_type, p.page).await
}

#[derive(Deserialize)]
pub struct PostCreateParams {
	message: String,
	tags: String
}

async fn _channel_feed( g: web::Data<Arc<Globals>>, id: &str, id_type: &str, page: u32 ) -> error::Result<HttpResponse> {
	const PAGE_SIZE: u64 = 10;

	let (address, public_key, local) = match id_type {
		"address" => (id.to_owned(), PublicKey::from_string( &id ).unwrap(), false ),
		"ego" => {
			let mut identity_service = gnunet::identity::Handle::connect( g.gnunet.clone() ).await
				.map_err(|_| error::ErrorInternalServerError("Gnunet identity service not available."))?;
			let priv_key = identity_service.lookup( &id ).await.expect("unexpected gnunet error").expect("ego not found");
			let public_key = priv_key.extract_public().expect("unable to extract public key");
			( public_key.to_string(), public_key, true )
		},
		_ => {
			panic!("ID type not supported");
		}
	};

	let mut context = tera::Context::new();
	context.insert("address", &address);

	let mut db = persistence::Handle::connect( g.gnunet.clone() ).await.map_err(|e| persistence::Error::Database(e))?
		.get_channel( &public_key ).await?.expect("unknown channel")
		.get_timeline( &public_key ).await?.expect("unknown publisher");
	let posts = db.list_posts( (page as u64 - 1)*PAGE_SIZE, PAGE_SIZE as _ ).await?;

	let post_previews = load_post_previews( &db, &*posts ).await?;
	context.insert("feed", &post_previews);

	let template_file = if local { "blog/own-feed.html" } else { "blog/feed.html" };

	let html = g.tera.render(template_file, &context)
		.map_err(|e| { eprintln!("Template error: {}", e); error::ErrorInternalServerError("Template error") } )?;
	Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

#[get("/channel/feed/{id_type}/{id}")]
pub async fn channel_feed_first( g: web::Data<Arc<Globals>>, p: web::Path<BlogFeedIdParams>) -> error::Result<HttpResponse> {
	_channel_feed( g, &p.id, &p.id_type, 1 ).await
}

/*#[post("/channel/feed/{id_type}/{id}")]
pub async fn channel_feed_post( g: web::Data<Arc<Globals>>, p: web::Path<BlogFeedIdParams>, f: web::Form<PostCreateParams>) -> error::Result<HttpResponse> {
	
	if p.id_type != "ego" {
		panic!("Posts can only be created by local ego's!");
	}

	let mut identity_service = gnunet::identity::Handle::connect( g.gnunet.clone() ).await
		.map_err(|_| error::ErrorInternalServerError("Gnunet identity service not available."))?;
	let private_key = identity_service.lookup( &p.id ).await.expect("gnunet error").expect("ego not found");
	drop( identity_service );

	// Create post
	let blog_address = private_key.extract_public().unwrap();
	let blog = persistence::Handle::connect( g.gnunet.clone() ).await?.get_channel( &blog_address ).await?;

	let tags: Vec<String> = f.tags.split_whitespace().map(|x| x.to_owned()).collect();
	let post_info = PostInfo {
		tags,new
		publish_timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as _
	};
	blog.create_post( &private_key, &f.message, post_info ).await?;

	_channel_feed( g, &p.id, &p.id_type, 1 ).await
}*/



impl From<persistence::Error> for actix_web::Error {
	fn from( other: persistence::Error ) -> Self {
		eprintln!("Persistence error: {}", other);
		error::ErrorInternalServerError("Internal server error occurred")
	}
}