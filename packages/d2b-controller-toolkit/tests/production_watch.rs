mod support;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use d2b_contracts::v3::{ResourceRef, ResourceTypeName};
use d2b_controller_toolkit::{
    OperationContext, PendingQueue, PriorityLane, QueueHint, ResourceKey, TriggerReason, TriggerSet,
};
use d2b_resource_api::watch::{
    WatchFrame, WatchPumpError, WatchService, WatchSink, WatchSinkError,
};
use serde_json::Value;

struct ControllerSink {
    frames: Mutex<Vec<WatchFrame>>,
}

impl ControllerSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            frames: Mutex::new(Vec::new()),
        })
    }
}

impl WatchSink for ControllerSink {
    #[allow(clippy::manual_async_fn)]
    fn send(
        &self,
        frame: WatchFrame,
    ) -> impl std::future::Future<Output = Result<(), WatchSinkError>> + Send {
        async move {
            let mut frames = self.frames.lock().unwrap();
            frames.push(frame);
            if frames.len() == 2 {
                Err(WatchSinkError::Closed)
            } else {
                Ok(())
            }
        }
    }
}

struct ControllerConsumer {
    queue: PendingQueue,
}

impl ControllerConsumer {
    fn new() -> Self {
        Self {
            queue: PendingQueue::new(8, 2),
        }
    }

    fn consume(&self, frame: &WatchFrame) {
        let payload: Value =
            serde_json::from_slice(frame.payload()).expect("watch frame has canonical JSON");
        let has_owner_hint = payload["ownerHints"]
            .as_array()
            .is_some_and(|hints| !hints.is_empty());
        for entry in payload["entries"]
            .as_array()
            .expect("watch entries are an array")
        {
            let resource_type = ResourceTypeName::parse(entry["resource_type"].as_str().unwrap())
                .expect("watch entry has a valid resource type");
            let resource_name = entry["resource_name"].as_str().unwrap();
            let resource_uid =
                d2b_contracts::v3::ResourceUid::parse(entry["resource_uid"].as_str().unwrap())
                    .expect("watch entry has an immutable UID");
            let resource_ref = ResourceRef::new(
                resource_type,
                d2b_contracts::v3::ResourceName::parse(resource_name)
                    .expect("watch entry has a valid resource name"),
            );
            let reason = if has_owner_hint {
                TriggerReason::OwnedResourceChanged
            } else {
                TriggerReason::SpecGenerationChanged
            };
            let key = ResourceKey::new(
                ZoneId::parse("work").expect("valid Zone"),
                resource_ref,
                resource_uid,
            );
            let operation = OperationContext::new(
                format!("watch-{}", frame.revision().get()),
                format!("watch-key-{}", frame.revision().get()),
                format!("watch-correlation-{}", frame.revision().get()),
                None,
            )
            .expect("bounded watch operation");
            self.queue
                .push(
                    QueueHint::new(
                        key,
                        frame.revision(),
                        TriggerSet::new([reason]),
                        PriorityLane::Ordinary,
                        operation,
                    )
                    .expect("watch entry maps to a valid controller hint"),
                )
                .expect("controller queue has bounded test capacity");
        }
    }
}

use d2b_contracts::v3::ZoneId;

#[tokio::test]
async fn production_watch_frames_drive_controller_queue_after_store_commit() {
    let (_directory, store, issuer) = support::provision_store().await;
    let first_revision = support::commit_host(&store, &issuer, "owner", None, "owner").await;
    let service = WatchService::new(Arc::clone(&store));
    let mut watch = service
        .open(support::watch_request(0, 4))
        .await
        .expect("open production watch");
    let second_revision =
        support::commit_host(&store, &issuer, "child", Some("Host/owner"), "child").await;
    let sink = ControllerSink::new();
    let sink_for_task = Arc::clone(&sink);
    let pump = tokio::spawn(async move { watch.pump_to(sink_for_task.as_ref()).await });
    let result = tokio::time::timeout(Duration::from_secs(1), pump)
        .await
        .expect("controller watch pump completes")
        .expect("controller watch task joins");
    assert_eq!(result, Err(WatchPumpError::Sink(WatchSinkError::Closed)));

    let consumer = ControllerConsumer::new();
    let frames = sink.frames.lock().unwrap();
    assert_eq!(frames.len(), 2);
    for frame in frames.iter() {
        consumer.consume(frame);
    }
    let first = consumer
        .queue
        .pop_ready()
        .expect("first watch item is ready");
    assert_eq!(first.high_water_revision(), first_revision);
    assert!(
        first
            .reasons()
            .contains(TriggerReason::SpecGenerationChanged)
    );
    consumer.queue.finish(first.key()).unwrap();
    let second = consumer
        .queue
        .pop_ready()
        .expect("second watch item is ready");
    assert_eq!(second.high_water_revision(), second_revision);
    assert!(
        second
            .reasons()
            .contains(TriggerReason::OwnedResourceChanged)
    );
    consumer.queue.finish(second.key()).unwrap();
    assert!(consumer.queue.pop_ready().is_none());
    drop(frames);
    assert_eq!(store.watch_signals().unwrap().budget_used, 0);
}
