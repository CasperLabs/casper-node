use futures::{future, Future, FutureExt};
use tokio::{
    select,
    sync::{broadcast, mpsc, oneshot},
    task,
};
use tracing::{info, trace};
use wheelbuf::WheelBuf;

use casper_types::ProtocolVersion;

use super::{
    sse_server::{BroadcastChannelMessage, NewSubscriberInfo, ServerSentEvent},
    Config, SseData,
};

/// Run the HTTP server.
///
/// * `server_with_shutdown` is the actual server as a future which can be gracefully shut down.
/// * `server_shutdown_sender` is the channel by which the server will be notified to shut down.
/// * `data_receiver` will provide the server with local events which should then be sent to all
///   subscribed clients.
/// * `broadcaster` is used by the server to send events to each subscribed client after receiving
///   them via the `data_receiver`.
/// * `new_subscriber_info_receiver` is used to notify the server of the details of a new client
///   having subscribed to the event stream.  It allows the server to populate that client's stream
///   with the requested number of historical events.
pub(super) async fn run(
    config: Config,
    api_version: ProtocolVersion,
    server_with_shutdown: impl Future<Output = ()> + Send + 'static,
    server_shutdown_sender: oneshot::Sender<()>,
    mut data_receiver: mpsc::UnboundedReceiver<SseData>,
    broadcaster: broadcast::Sender<BroadcastChannelMessage>,
    mut new_subscriber_info_receiver: mpsc::UnboundedReceiver<NewSubscriberInfo>,
) {
    let server_joiner = task::spawn(server_with_shutdown);

    // Initialize the index and buffer for the SSEs.
    let mut event_index = 0_u32;
    let mut buffer = WheelBuf::new(vec![
        ServerSentEvent::initial_event(api_version);
        config.event_stream_buffer_length as usize
    ]);

    // Start handling received messages from the two channels; info on new client subscribers and
    // incoming events announced by node components.
    let event_stream_fut = async {
        loop {
            select! {
                maybe_new_subscriber = new_subscriber_info_receiver.recv() => {
                    if let Some(subscriber) = maybe_new_subscriber {
                        // First send the client the `ApiVersion` event.  We don't care if this
                        // errors - the client may have disconnected already.
                        let _ = subscriber
                            .initial_events_sender
                            .send(ServerSentEvent::initial_event(api_version));
                        // If the client supplied a "start_from" index, provide the buffered events.
                        // If they requested more than is buffered, just provide the whole buffer.
                        if let Some(start_index) = subscriber.start_from {
                            for event in buffer
                                .iter()
                                .skip_while(|event| event.id.unwrap() < start_index)
                            {
                                // As per sending `SSE_INITIAL_EVENT`, we don't care if this errors.
                                let _ = subscriber.initial_events_sender.send(event.clone());
                            }
                        }
                    }
                }

                maybe_data = data_receiver.recv() => {
                    match maybe_data {
                        Some(data) => {
                            // Buffer the data and broadcast it to subscribed clients.
                            trace!("Event stream server received {:?}", data);
                            let event = ServerSentEvent { id: Some(event_index), data };
                            buffer.push(event.clone());
                            let message = BroadcastChannelMessage::ServerSentEvent(event);
                            // This can validly fail if there are no connected clients, so don't log
                            // the error.
                            let _ = broadcaster.send(message);
                            event_index = event_index.wrapping_add(1);
                        }
                        None => {
                            // The data sender has been dropped - exit the loop.
                            info!("shutting down HTTP server");
                            break;
                        }
                    }
                }
            }
        }
    };

    // Wait for the event stream future to exit, which will only happen if the last `data_sender`
    // paired with `data_receiver` is dropped.  `server_joiner` will never return here.
    let _ = future::select(server_joiner, event_stream_fut.boxed()).await;

    // Kill the event-stream handlers, and shut down the server.
    let _ = broadcaster.send(BroadcastChannelMessage::Shutdown);
    let _ = server_shutdown_sender.send(());

    trace!("Event stream server stopped");
}
