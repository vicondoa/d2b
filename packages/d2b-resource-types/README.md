# `d2b-resource-types`

Shared declaration vocabulary for the v3 resource plane: `DriverDescriptor`,
`AllowedSources`, `ChildCreation`, `OperationDef`, `ServiceDecl`,
`StartupStep`, `ProviderDeclaration`, and the well-known resource type names.

## What belongs here

Types only. The crate declares the shapes a per-type provider crate builds
its declarations from, and the `OperationHandler` contract the declared
operations carry. It holds no resource-specific knowledge and no runtime
logic.

## What does not belong here

Drivers, spec decoders, driver factories, operation handlers, and effects
live in the per-type provider crates that declare them. Registration
mechanics, the driver registry, catalogs, and generated artifacts are
consumers of this vocabulary, not part of it.

## Placement and dependencies

Depends on `d2b-contracts-resource` and `d2b-resource-runtime` for the
canonical reference types and the driver contracts the declarations name.
Crates that implement a resource type depend on this crate; this crate never
depends on them.

## Build and test

```bash
bazel test //packages/d2b-resource-types:all-tests
```
