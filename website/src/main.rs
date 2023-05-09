use actix_files::NamedFile;
use actix_service::{fn_service, Service};
use actix_session::{storage::CookieSessionStore, SessionExt, SessionMiddleware};
use actix_web::{
    cookie,
    dev::{ServiceRequest, ServiceResponse},
    get,
    http::header::{ContentDisposition, DispositionType},
    middleware::Logger,
    web, App, HttpMessage, HttpRequest, HttpServer,
};
use aws_config;
use aws_sdk_s3::types::{
    builders::CreateBucketConfigurationBuilder, CreateBucketConfiguration, ObjectOwnership,
    OwnershipControls, OwnershipControlsRule, PublicAccessBlockConfiguration,
};
use futures_util::future::FutureExt;

use pokerbots::app::{api, login};

fn get_secret_key() -> cookie::Key {
    let key = std::env::var("SECRET_KEY").expect("SECRET_KEY must be set in .env");
    cookie::Key::from(key.as_bytes())
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "debug");
    env_logger::init();
    dotenvy::dotenv().ok();

    let aws_config = aws_config::load_from_env().await;

    let s3_client = web::Data::new(aws_sdk_s3::Client::new(&aws_config));

    s3_client
        .create_bucket()
        .bucket(std::env::var("PFP_S3_BUCKET").unwrap())
        .send()
        .await
        .unwrap();

    s3_client
        .delete_public_access_block()
        .bucket(std::env::var("PFP_S3_BUCKET").unwrap())
        .send()
        .await
        .unwrap();

    s3_client
        .put_bucket_ownership_controls()
        .bucket(std::env::var("PFP_S3_BUCKET").unwrap())
        .ownership_controls(
            OwnershipControls::builder()
                .rules(
                    OwnershipControlsRule::builder()
                        .object_ownership(ObjectOwnership::BucketOwnerPreferred)
                        .build(),
                )
                .build(),
        )
        .send()
        .await
        .unwrap();

    s3_client
        .put_bucket_cors()
        .bucket(std::env::var("PFP_S3_BUCKET").unwrap())
        .cors_configuration(
            aws_sdk_s3::types::CorsConfiguration::builder()
                .cors_rules(
                    aws_sdk_s3::types::CorsRule::builder()
                        .allowed_headers("*")
                        .allowed_methods("PUT")
                        .allowed_origins("*")
                        .build(),
                )
                .build(),
        )
        .send()
        .await
        .unwrap();

    // Generate the list of routes in your App
    HttpServer::new(move || {
        let session_middleware =
            SessionMiddleware::builder(CookieSessionStore::default(), get_secret_key())
                .cookie_secure(true)
                .build();
        let mut hbars = handlebars::Handlebars::new();
        hbars.set_strict_mode(true);
        hbars
            .register_templates_directory(".hbs", "templates")
            .expect("Failed to load templates");

        App::new()
            .wrap_fn(|req, srv| {
                let user_data = login::get_user_data(&req.get_session());
                let team_data = login::get_team_data(&req.get_session());
                req.extensions_mut().insert(user_data);
                req.extensions_mut().insert(team_data);
                log::debug!("{}", req.uri());
                srv.call(req).map(|res| res)
            })
            .app_data(s3_client.clone())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(session_middleware)
            .route("/api/login", web::get().to(login::handle_login))
            .service(login::login_provider)
            .service(api::manage_team::create_team)
            .service(api::manage_team::delete_team)
            .service(api::manage_team::leave_team)
            .service(api::manage_team::make_invite)
            .service(api::manage_team::pfp_upload_url)
            .service(api::manage_team::join_team)
            .service(api::manage_team::cancel_invite)
            .service(api::data::my_account)
            .service(api::data::server_message)
            .service(api::data::my_team)
            .service(api::data::pfp_url)
            .service(api::signout::signout)
            // All remaining paths go to /app/dist, and fallback to index.html for client side routing
            .service(
                actix_files::Files::new("/", "app/dist/")
                    .index_file("/index.html")
                    .default_handler(fn_service(|req: ServiceRequest| async {
                        let (req, _) = req.into_parts();

                        let f = NamedFile::open_async("app/dist/index.html")
                            .await?
                            .into_response(&req);
                        Ok(ServiceResponse::new(req, f))
                    })),
            )

        //.wrap(middleware::Compress::default())
    })
    .workers(8)
    .bind(("0.0.0.0", 3000))?
    .run()
    .await
}
