#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
component_dir="$repo_root/windows-native-audio-cable"
keys_dir="$repo_root/keys"
powershell_exe="/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
stage_dir="/mnt/c/ProgramData/VoxDriverSigning"

required=(
  "$keys_dir/vox-native-audio-cable.cer"
  "$keys_dir/vox-native-audio-cable.pfx"
  "$keys_dir/pfx-password.txt"
  "$component_dir/prepare-signing.ps1"
  "$powershell_exe"
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

mkdir -p "$stage_dir"
cleanup_stage() {
  rm -f -- \
    "$stage_dir/vox-native-audio-cable.cer" \
    "$stage_dir/vox-native-audio-cable.pfx" \
    "$stage_dir/pfx-password.txt" \
    "$stage_dir/windows-thumbprint.txt"
  rmdir --ignore-fail-on-non-empty "$stage_dir" 2>/dev/null || true
}
trap cleanup_stage EXIT

cp -- "$keys_dir/vox-native-audio-cable.cer" "$stage_dir/"
cp -- "$keys_dir/vox-native-audio-cable.pfx" "$stage_dir/"
cp -- "$keys_dir/pfx-password.txt" "$stage_dir/"

ps_script="$(to_windows_path "$component_dir/prepare-signing.ps1")"
certificate="$(to_windows_path "$stage_dir/vox-native-audio-cable.cer")"
pfx="$(to_windows_path "$stage_dir/vox-native-audio-cable.pfx")"
password="$(to_windows_path "$stage_dir/pfx-password.txt")"
thumbprint_output="$(to_windows_path "$stage_dir/windows-thumbprint.txt")"

"$powershell_exe" -NoLogo -NoProfile -Command \
  "\$source = [IO.File]::ReadAllText('$ps_script'); & ([ScriptBlock]::Create(\$source)) -CertificatePath '$certificate' -PfxPath '$pfx' -PasswordPath '$password' -ThumbprintOutputPath '$thumbprint_output'"

cp -- "$stage_dir/windows-thumbprint.txt" "$keys_dir/windows-thumbprint.txt"
chmod 600 "$keys_dir/windows-thumbprint.txt"

printf '\nWindows signing identity prepared. Thumbprint:\n'
cat "$keys_dir/windows-thumbprint.txt"
