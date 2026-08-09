#!/usr/bin/env python3
"""Exercise Vox's seven local compositions against the real model services."""

from __future__ import annotations

import argparse
import json
import mimetypes
import time
import urllib.request
import uuid
import wave
from pathlib import Path
from typing import Any


def json_post(url: str, payload: dict[str, Any], timeout: int = 300) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)


def multipart_post(
    url: str,
    fields: dict[str, object],
    files: dict[str, Path],
    timeout: int = 300,
) -> tuple[bytes, dict[str, str]]:
    boundary = f"vox-{uuid.uuid4().hex}"
    body = bytearray()
    for name, value in fields.items():
        body.extend(f"--{boundary}\r\n".encode())
        body.extend(f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode())
        body.extend(str(value).encode("utf-8"))
        body.extend(b"\r\n")
    for name, path in files.items():
        body.extend(f"--{boundary}\r\n".encode())
        body.extend(
            f'Content-Disposition: form-data; name="{name}"; filename="{path.name}"\r\n'.encode()
        )
        mime = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
        body.extend(f"Content-Type: {mime}\r\n\r\n".encode())
        body.extend(path.read_bytes())
        body.extend(b"\r\n")
    body.extend(f"--{boundary}--\r\n".encode())
    request = urllib.request.Request(
        url,
        data=bytes(body),
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read(), dict(response.headers.items())


def transcribe_detect(
    whisper: str, translator: str, audio: Path, mode: str, candidate_tokens: int
) -> tuple[str, str, dict[str, Any]]:
    candidate_body, _ = multipart_post(
        f"{whisper}/api/transcribe-candidates",
        {"operation": mode, "max_new_tokens": candidate_tokens},
        {"file": audio},
    )
    candidate_response = json.loads(candidate_body)
    arbitration = json_post(
        f"{translator}/arbitrate",
        {"candidates": candidate_response["candidates"]},
    )
    language = arbitration["language"]
    transcript_body, _ = multipart_post(
        f"{whisper}/api/transcribe",
        {
            "operation": mode,
            "language": language,
            "word_timestamps": "false",
            "max_new_tokens": 256,
        },
        {"file": audio},
    )
    transcript_response = json.loads(transcript_body)
    text = transcript_response["results"][mode]["text"].strip()
    if not text:
        raise RuntimeError("full Crisper pass returned empty text")
    return text, language, arbitration


def translate(translator: str, text: str, target: str) -> str:
    value = json_post(
        f"{translator}/translate",
        {"text": text, "target_language": target},
    )["translation"].strip()
    if not value:
        raise RuntimeError("translator returned empty text")
    return value


def synthesize(
    longcat: str,
    text: str,
    reference_audio: Path,
    reference_text: str,
    output: Path,
) -> dict[str, Any]:
    body, _ = multipart_post(
        f"{longcat}/api/synthesize",
        {
            "text": text,
            "prompt_text": reference_text,
            "steps": 16,
            "guidance_strength": 4.0,
            "guidance_method": "apg",
            "seed": 1024,
            "duration_scale": 1.0,
        },
        {"prompt_audio": reference_audio},
        timeout=600,
    )
    output.write_bytes(body)
    with wave.open(str(output), "rb") as wav:
        frames = wav.getnframes()
        rate = wav.getframerate()
        channels = wav.getnchannels()
    if frames <= 0 or rate <= 0:
        raise RuntimeError(f"LongCat returned invalid WAV: {output}")
    return {
        "file": str(output),
        "seconds": round(frames / rate, 3),
        "sample_rate": rate,
        "channels": channels,
        "bytes": len(body),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--expected-language", default="")
    parser.add_argument("--text", default="Hey, mein Freund, ich weiß die ganze Arbeit, die du leistest, wirklich zu schätzen.")
    parser.add_argument("--target", default="English")
    parser.add_argument("--direct-tts-text", default="I truly appreciate all the work you do.")
    parser.add_argument("--reference-audio", type=Path, required=True)
    parser.add_argument("--reference-text", required=True)
    parser.add_argument("--whisper", default="http://127.0.0.1:8172")
    parser.add_argument("--translator", default="http://127.0.0.1:8176")
    parser.add_argument("--longcat", default="http://127.0.0.1:8230")
    parser.add_argument("--mode", choices=("intended", "verbatim"), default="intended")
    parser.add_argument("--candidate-tokens", type=int, default=24)
    parser.add_argument("--output", type=Path, default=Path(".runtime/route-acceptance"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)

    started = time.monotonic()
    sound_text, language, arbitration = transcribe_detect(
        args.whisper, args.translator, args.audio, args.mode, args.candidate_tokens
    )
    if args.expected_language and language != args.expected_language:
        raise RuntimeError(
            f"expected detected language {args.expected_language!r}, got {language!r}"
        )
    translated_sound = translate(args.translator, sound_text, args.target)
    translated_text = translate(args.translator, args.text, args.target)

    routes = {
        "1_sound_stt_text": {"text": sound_text, "language": language},
        "2_sound_stt_translate_text": {"text": translated_sound, "target": args.target},
        "3_sound_stt_tts": synthesize(
            args.longcat,
            sound_text,
            args.reference_audio,
            args.reference_text,
            args.output / "route-3.wav",
        ),
        "4_sound_stt_translate_tts": synthesize(
            args.longcat,
            translated_sound,
            args.reference_audio,
            args.reference_text,
            args.output / "route-4.wav",
        ),
        "5_text_translate_text": {"text": translated_text, "target": args.target},
        "6_text_translate_tts": synthesize(
            args.longcat,
            translated_text,
            args.reference_audio,
            args.reference_text,
            args.output / "route-6.wav",
        ),
        "7_text_tts": synthesize(
            args.longcat,
            args.direct_tts_text,
            args.reference_audio,
            args.reference_text,
            args.output / "route-7.wav",
        ),
    }
    report = {
        "passed": True,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "detect_lane": {
            "selected_language": language,
            "finalist_count": arbitration.get("finalist_count"),
            "fragment_arbiter_output": arbitration.get("arbiter_output"),
            "ambiguity_output": arbitration.get("ambiguity_output"),
        },
        "routes": routes,
    }
    report_path = args.output / "report.json"
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps(report, indent=2, ensure_ascii=False))
    print(f"Report: {report_path}")


if __name__ == "__main__":
    main()
