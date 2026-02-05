# Builder pattern conventions

This guide describes how to handle type-transitioning builder methods in
Wireframe. When a method changes a generic parameter, struct update syntax
(`..self`) cannot be used, so the builder must be reconstructed explicitly.

## Choosing an approach

Use a helper method when:

- The builder has many fields (roughly ten or more).
- Type transitions update multiple related fields.
- The same reconstruction logic appears in more than one method.

Use a macro when:

- The builder has a small, stable field set (single digits).
- Each method updates a single field.
- The reconstruction is a straightforward field copy.

## Current patterns

- `WireframeApp::rebuild_with_params` centralizes reconstruction for the
  13-field server builder and keeps coordinated updates for serializer, codec,
  protocol, and fragmentation together.
- `builder_field_update!` in `src/client/builder.rs` covers the five-field
  client builder, where each type change updates one field at a time.
