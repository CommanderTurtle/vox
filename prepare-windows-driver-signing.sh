#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
component_dir="$repo_root/windows-native-audio-cable"
keys_dir="$repo_root/keys"
powershell_exe="/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"
stage_dir="/mnt/c/ProgramData/VoxDriverSigning"

key_bundle=(
  "$keys_dir/vox-native-audio-cable.cer"
  "$keys_dir/vox-native-audio-cable.crt"
  "$keys_dir/vox-native-audio-cable.key"
  "$keys_dir/vox-native-audio-cable.pfx"
  "$keys_dir/pfx-password.txt"
)

existing_key_files=0
for file in "${key_bundle[@]}"; do
  if [[ -e "$file" ]]; then
    ((existing_key_files += 1))
  fi
done

if (( existing_key_files == 0 )); then
  umask 077
  mkdir -p "$keys_dir"
  password="$(openssl rand -hex 32)"
  printf '%s' "$password" > "$keys_dir/pfx-password.txt"
  openssl req -x509 -newkey rsa:3072 -sha256 -days 3650 -nodes \
    -keyout "$keys_dir/vox-native-audio-cable.key" \
    -out "$keys_dir/vox-native-audio-cable.crt" \
    -subj '/CN=Vox Native Audio Cable' \
    -addext 'basicConstraints=critical,CA:FALSE' \
    -addext 'keyUsage=critical,digitalSignature' \
    -addext 'extendedKeyUsage=codeSigning'
  openssl x509 \
    -in "$keys_dir/vox-native-audio-cable.crt" \
    -outform DER \
    -out "$keys_dir/vox-native-audio-cable.cer"
  openssl pkcs12 -export \
    -out "$keys_dir/vox-native-audio-cable.pfx" \
    -inkey "$keys_dir/vox-native-audio-cable.key" \
    -in "$keys_dir/vox-native-audio-cable.crt" \
    -passout "file:$keys_dir/pfx-password.txt" \
    -CSP 'Microsoft Enhanced RSA and AES Cryptographic Provider' \
    -LMK
  (
    cd "$keys_dir"
    sha256sum \
      vox-native-audio-cable.crt \
      vox-native-audio-cable.cer \
      vox-native-audio-cable.pfx > SHA256SUMS
  )
  chmod 700 "$keys_dir"
  chmod 600 "$keys_dir"/*
  unset password
  printf 'Generated a new local code-signing bundle in %s\n' "$keys_dir"
elif (( existing_key_files != ${#key_bundle[@]} )); then
  printf 'The key bundle is incomplete. Refusing to overwrite existing signing material in %s\n' "$keys_dir" >&2
  exit 1
else
  printf 'Reusing the existing local code-signing bundle in %s\n' "$keys_dir"
fi

required=(
  "${key_bundle[@]}"
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
