# Listenbrainz Playlist Uploader

This is a helpful tool to upload all of your M3U playlists to Listenbrainz, the open source music scrobbler and history keeper. It also provides the ability to leave feedback on the songs in those playlists.

Tags are automatically read from the linked files and then matched with an ID through the Listenbrainz service. As such, the files in the playlist must have readable tags to work.

The token for the Listenbrainz account is required, and must be placed in a `config.toml` file under the key `user_token`. See the example configuration file for details.

**Usage:** `listenbrainz_playlist_uploader [OPTIONS] <FILE> <PLAYLIST_NAME>`

### **Arguments:**

* `<FILE>`
* `<PLAYLIST_NAME>`

### **Options:**

* `-c`, `--config <CONFIG>`
  - Default value: `./config.toml`
* `-f`, `--feedback <FEEDBACK>`
  - Possible values: `love`, `hate`, `neutral`
  - Feedback is applied to all songs in the playlist.
  - If not supplied, feedback is not changed.
* `-p`, `--public`
  - Default value: `false`
  - Possible values: `true`, `false`
  - Determines whether the playlist will be publicly visible or not.
* `-v`, `--verbose` — Increase logging verbosity
* `-q`, `--quiet` — Decrease logging verbosity
* `-d`, `--duplicate-action <DUPLICATE_ACTION>`
  - Default value: `none`
  - Possible values: `none`, `overwrite`, `number`, `abort`
  - What to do when there is already a playlist by the same name on your account.
    - If you choose, `number`, a number will be appended to the end of the playlist name.
    - If you choose none, two playlists will have the same name but separate IDs.
* `-n`, `--no-confirm`
  - Default value: `false`
  - Possible values: `true`, `false`
  - Disables any interaction in the program.

### Things to Do

- Make the song search better so that more songs are matched.
- Read the Listenbrainz rate limiting dynamically to be more efficient.
- Add pagination for playlist finding.

<hr/>

<small><i>
    This document was partially generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
