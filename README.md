# rterminalfy

Klien SSH desktop untuk individu, ditulis sepenuhnya dalam Rust. Berjalan di Windows dan macOS.

Versi: 0.1.0

## Ciri

| Ciri | Status |
| --- | --- |
| Pengurusan host, kumpulan dan carian | Siap |
| Tema (lalai: teks hijau, latar hitam) | Siap |
| Skrin About dengan senarai library, versi dan pautan | Siap |
| Terminal SSH dan jump server | Dalam pembangunan |
| SFTP dan port forwarding | Dalam pembangunan |
| Keychain: jana kunci, identiti, sijil, FIDO2 | Dalam pembangunan |

## Bina dan jalankan

Perlukan [Rust](https://rustup.rs) 1.92 atau lebih baharu.

```
cargo run --release
```

## Ujian

```
cargo test
```

## Data

Host dan tetapan disimpan sebagai JSON dalam folder konfigurasi pengguna
(`%APPDATA%\rterminalfy\config\data.json` di Windows,
`~/Library/Application Support/rterminalfy/data.json` di macOS).
Kata laluan dan kunci peribadi tidak pernah ditulis ke fail ini.

## Library

Setiap library yang digunakan disenaraikan dalam skrin About bersama versi
dan laman webnya. Versi dibaca daripada `Cargo.lock` semasa binaan, dan ujian
gagal jika ada library dalam `Cargo.toml` yang belum disenaraikan.
