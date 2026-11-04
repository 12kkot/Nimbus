/* Opus already comes with LiveKit's WebRTC library. Songbird's libopus_sys
 * would build and link a second copy (duplicate symbols), so it is pointed at
 * this empty lib/opus.lib instead (see .cargo/config.toml).
 * Rebuild: cl /nologo /c opus_stub.c && lib /nologo /OUT:lib\opus.lib opus_stub.obj */
int nimbus_opus_stub;
