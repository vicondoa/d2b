### Fixed

- The provider session loop now compares the bound controller route inside the route mutex lock scope instead of cloning the route binding out of the lock on every frame, dropping the per-frame `AuthenticatedSessionRouteBinding` clone.
- `SharedProviderEffectRequest` now borrows the row's canonical spec document from the driver's spec envelope instead of cloning it into every reconcile and delete request, removing one full-spec allocation per pass.