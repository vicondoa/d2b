### Fixed

- The network family's declared-service intent source loads a fresh bundle
  resolver per invocation again, as the pre-move adapter did. A replaced bundle
  is now picked up without a daemon restart and a tampered or unreadable bundle
  fails the reconcile closed, restoring convergence behaviour the conversion
  had changed.