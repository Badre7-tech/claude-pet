// Downloads the speech model used for "Hey Pyrosan" (whisper.cpp, small, quantized) into
// src-tauri/resources/models, once, and checks it hasn't been tampered with.
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream, existsSync, mkdirSync, renameSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const URL = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin";
const SHA256 = "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb";
const file = join(dirname(fileURLToPath(import.meta.url)), "../src-tauri/resources/models/ggml-small-q5_1.bin");

async function sha256(path) {
  const hash = createHash("sha256");
  await pipeline(createReadStream(path), hash);
  return hash.digest("hex");
}

if (existsSync(file) && (await sha256(file)) === SHA256) process.exit(0);

mkdirSync(dirname(file), { recursive: true });
console.log("downloading the speech model (190 MB)…");
const response = await fetch(URL);
if (!response.ok) throw new Error(`speech model download failed: HTTP ${response.status}`);
const partial = `${file}.part`;
await pipeline(Readable.fromWeb(response.body), createWriteStream(partial));
const got = await sha256(partial);
if (got !== SHA256) {
  rmSync(partial);
  throw new Error(`speech model checksum mismatch: ${got}`);
}
renameSync(partial, file);
console.log("speech model ready");
