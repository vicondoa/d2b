# Integration fixtures

The clipboard integration lane uses fake display host-clipboard and bridge
services. It covers host capture, Guest capture, picker coordination,
backpressure, FD rejection, audit queue pressure, Guest lock/destroy purge,
host-only mode, and cross-Zone denial. No live compositor, filesystem bridge,
or D-Bus address is used.
