#![allow(dead_code)]

use futures_util::future::{BoxFuture, FutureExt, IntoStream};
use futures_util::stream;
use futures_util::stream::{Map, Select, Stream, StreamExt};
use log::Logger;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{env, mem};
use twilight_gateway::shard::{Events, ShardStartError};
use twilight_gateway::{Event, Shard};
use twilight_model::gateway::payload::outgoing::identify::IdentifyProperties;
use twilight_model::gateway::payload::outgoing::update_presence::UpdatePresencePayload;
use twilight_model::gateway::presence::Status;
use twilight_model::gateway::Intents;

mod log;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum GatewayEvent {
    Started,
    StartError(ShardStartError),
    Event(Event),
}

type MapEvent = fn(Event) -> GatewayEvent;
type MapStart = fn(Result<(), ShardStartError>) -> GatewayEvent;
type StartFuture<'a> = BoxFuture<'a, Result<(), ShardStartError>>;
type GatewayStream = Select<Map<Events, MapEvent>, Map<IntoStream<StartFuture<'static>>, MapStart>>;

fn map_event(event: Event) -> GatewayEvent {
    GatewayEvent::Event(event)
}

fn map_start(result: Result<(), ShardStartError>) -> GatewayEvent {
    match result {
        Ok(()) => GatewayEvent::Started,
        Err(error) => GatewayEvent::StartError(error),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    // Setup logging to `$CARGO_MANIFEST_DIR/log`.
    let logger = Logger::new()?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(logger);

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(non_blocking);

    tracing::subscriber::set_global_default(subscriber.finish())?;

    let token = env::var("DISCORD_TOKEN")?;
    let intents = Intents::GUILD_MESSAGES;

    unsafe fn change_ref<'a, 'b, T>(a: &'a T) -> &'b T {
        mem::transmute(a)
    }

    pub struct Gateway {
        shard: Shard,
        stream: GatewayStream,
        _pin: PhantomPinned,
    }

    impl Gateway {
        pub fn new(shard: Shard, events: Events) -> Self {
            let shard_ref: &'static Shard = unsafe { change_ref(&shard) };
            let start_stream = shard_ref
                .start()
                .boxed()
                .into_stream()
                .map(map_start as MapStart);

            let events_stream = events.map(map_event as MapEvent);
            let stream = stream::select(events_stream, start_stream);
            let _pin = PhantomPinned;

            Self {
                shard,
                stream,
                _pin,
            }
        }

        fn project<'pin>(self: Pin<&'pin mut Self>) -> GatewayProjection<'pin> {
            unsafe {
                let Self {
                    shard,
                    stream,
                    _pin,
                } = self.get_unchecked_mut();

                GatewayProjection {
                    shard,
                    stream: Pin::new_unchecked(stream),
                }
            }
        }
    }

    struct GatewayProjection<'pin> {
        shard: &'pin mut Shard,
        stream: Pin<&'pin mut GatewayStream>,
    }

    impl Stream for Gateway {
        type Item = GatewayEvent;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.project();

            this.stream.poll_next(cx)
        }
    }

    pub const BROWSER: &str = "Discord Client";
    pub const RELEASE_CHANNEL: &str = "canary";
    pub const CLIENT_VERSION: &str = "0.0.133";

    pub const OS: &str = "Linux";
    pub const OS_ARCH: &str = "x64";
    pub const OS_VERSION: &str = "5.16.10"; // stable

    pub const SYSTEM_LOCALE: &str = "en-US";

    pub const WINDOW_MANAGER: &str = "unknown,unknown";

    let (shard, events) = Shard::builder(token, intents)
        .gateway_url(Some("wss://gateway.discord.gg".to_string()))
        .identify_properties(IdentifyProperties {
            browser: Box::from(BROWSER),
            client_version: Box::from(CLIENT_VERSION),
            os: Box::from(OS),
            os_arch: Box::from(OS_ARCH),
            os_version: Box::from(OS_VERSION),
            release_channel: Box::from(RELEASE_CHANNEL),
            system_locale: Box::from(SYSTEM_LOCALE),
            window_manager: Box::from(WINDOW_MANAGER),
        })
        .presence(UpdatePresencePayload::new(
            vec![],
            false,
            None,
            Status::DoNotDisturb,
        )?)
        .build();

    let mut stream = Box::pin(Gateway::new(shard, events));

    while let Some(event) = stream.next().await {
        println!("{:?}", event);
    }

    Ok(())
}
