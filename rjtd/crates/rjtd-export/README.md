# rjtd-export

Experimental plain text exporter for OpenJTD documents.

`rjtd-export` is the export layer of
[OpenJTD](https://github.com/KimEJ/OpenJTD). It consumes `rjtd-model::Document`;
it does not parse source files directly.

## Developer preview

Version 0.0.1 is an experimental developer preview. All public APIs and output
details may change in any later 0.0.x release.

## Exports

- Plain text through `to_plain_text`.

## Public API

`to_plain_text` takes `&rjtd_model::Document` and returns a `String`.

## Example

```rust,no_run
let bytes = std::fs::read("document.jtd")?;
let document = rjtd_model::parse_document(&bytes)?;
let text = rjtd_export::to_plain_text(&document);
println!("{text}");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Output limits

Text output uses the currently decoded document model. The exporter does not
promise complete feature coverage or lossless round trips for 0.0.1.

## License

Apache-2.0.
