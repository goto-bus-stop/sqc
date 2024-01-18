# sqc
A SQLite CLI with syntax highlighting and pretty tables by default.

This is not a full replacement for the official SQLite CLI.
I use it for just querying and updating databases, and use the official one if I need to do something more advanced.

## Syntax
The interactive CLI works similarly to the official SQLite CLI, but not exactly the same. Input is interpreted as
SQL statements except dot commands. Use `.help` for a list of available commands and documentation.

## Extensions
`sqc` includes the CSV vtable extension.

It also has additional functions for inspecting data:
- `fmt_byte_size(col)` - given an integer number of bytes, format it as a human-readable string (eg. `12 kB`)

## License
Licensed under either of Apache License, Version 2.0 or MIT license at your option.
