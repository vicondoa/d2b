# Integration fixtures

The notification integration lane uses fake enrolled ComponentSession and
desktop effect ports. It covers Guest source admission, bounded stream
delivery, display dependency readiness, observer projection, action invocation,
drain, and restart invalidation. Notification content remains in the
presentation-only fake and never enters evidence or diagnostics.
