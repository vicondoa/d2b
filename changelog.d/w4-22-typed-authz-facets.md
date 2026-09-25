### Fixed

- The broker operation catalog now carries the authorization facets as the typed `SecretAccess`, `BrokerRequirement`, and `AuditMode` enums instead of string literals, matching the sibling authz view and closing the case drift between the two generated forms.