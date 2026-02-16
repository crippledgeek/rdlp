//! Optional event fan-out for multi-subscriber scenarios.
//!
//! The default [`DownloadHandle`](crate::handle::DownloadHandle) provides a
//! single-consumer `mpsc` channel. Use [`EventBus`] when multiple consumers
//! need to receive all events (e.g., Tauri event bridge + progress logger).

use crate::events::Event;
use tokio::sync::broadcast;

/// Fan-out event bus backed by `tokio::sync::broadcast`.
///
/// Create subscribers before sending events. Events sent before
/// a subscriber is created are lost (broadcast channel semantics).
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    /// Create a new event bus with the given channel capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of events buffered per subscriber.
    ///   Lagging subscribers lose oldest events. 256 is a good default.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Create a new subscriber that receives all future events.
    ///
    /// Call this **before** sending events to avoid missing them.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }

    /// Send a single event to all subscribers.
    ///
    /// Returns the number of subscribers that received the event.
    /// Returns 0 if no subscribers exist.
    #[must_use]
    pub fn send(&self, event: Event) -> usize {
        self.sender.send(event).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::DownloadId;

    #[tokio::test]
    async fn test_event_bus_single_subscriber() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        let id = DownloadId::next();
        let event = Event::Started {
            id,
            url: "https://example.com/video".into(),
        };

        let count = bus.send(event.clone());
        assert_eq!(count, 1);

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.download_id(), id);
    }

    #[tokio::test]
    async fn test_event_bus_multi_subscriber() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let id = DownloadId::next();
        let event = Event::Warning {
            id,
            message: "test warning".into(),
        };

        let count = bus.send(event.clone());
        assert_eq!(count, 2);

        let received1 = rx1.recv().await.expect("subscriber 1 should receive");
        let received2 = rx2.recv().await.expect("subscriber 2 should receive");
        assert_eq!(received1.download_id(), id);
        assert_eq!(received2.download_id(), id);
    }

    #[tokio::test]
    async fn test_event_bus_no_subscribers() {
        let bus = EventBus::new(16);

        let id = DownloadId::next();
        let event = Event::Cancelled { id };

        let count = bus.send(event);
        assert_eq!(count, 0);
    }
}
