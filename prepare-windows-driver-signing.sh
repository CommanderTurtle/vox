#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
component_dir="$repo_root/windows-native-audio-cable"
keys_dir="$repo_root/keys"

required=(
  "$keys_dir/vox-native-audio-cable.cer"
  "$keys_dir/vox-native-audio-cable.pfx"
  "$keys_dir/pfx-password.txt"
  "$component_dir/prepare-signing.ps1"
)
for file in "${required[@]}"; do
  if [[ ! -f "$file" ]]; then
    printf 'Missing required file: %s\n' "$file" >&2
    exit 1
  fi
done

to_windows_path() {
  wslpath -w "$1"
}

powershell.exe \
  -NoLogo \
  -NoProfile \
  -ExecutionPolicy Bypass \
  -File "$(to_windows_path "$component_dir/prepare-signing.ps1")" \
  -CertificatePath "$(to_windows_path "$keys_dir/vox-native-audio-cable.cer")" \
  -PfxPath "$(to_windows_path "$keys_dir/vox-native-audio-cable.pfx")" \
  -PasswordPath "$(to_windows_path "$keys_dir/pfx-password.txt")" \
  -ThumbprintOutputPath "$(to_windows_path "$keys_dir/windows-thumbprint.txt")"

printf '\nWindows signing identity prepared. Thumbprint:\n'
cat "$keys_dir/windows-thumbprint.txt"
