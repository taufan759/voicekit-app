# VoiceKit

Studio text-to-speech ala ElevenLabs untuk bikin voice over konten, dalam aplikasi desktop yang ringan. Mesin suaranya adalah **API resmi ElevenLabs** dengan model terbaru (**Eleven v4**, v4 Turbo, v3, dan lainnya) memakai API key akunmu sendiri.

## Fitur

- **Studio**: editor naskah dengan tombol audio tag v4 (`[excited]`, `[whispers]`, `[laughs]`, `[short pause]`, dan lain-lain) plus arahan bebas, misalnya `[said slowly like a storyteller]`.
- **Naskah panjang otomatis dipecah** sesuai batas karakter model, di batas kalimat, tanpa memotong audio tag. Tiap bagian mendapat konteks kalimat sebelum dan sesudahnya, lalu digabung jadi satu MP3.
- **Pengaturan suara**: stabilitas, kemiripan, gaya, kecepatan, speaker boost, seed, bahasa, dan format MP3.
- **Suara**: jelajahi suara default (gratis), suara kloningmu, dan suara yang kamu simpan dari Voice Library, lengkap dengan contoh audio. Suara **Fin** otomatis dipilih kalau ada di akunmu.
- **Kloning suaramu sendiri** (Instant Voice Clone) dari rekaman. Wajib mencentang pernyataan bahwa suara itu milikmu atau kamu punya izin.
- **Riwayat**: putar ulang, buka folder, atau pakai lagi naskah dan pengaturannya.
- **Kuota**: sisa karakter paketmu tampil di sidebar.

## Privasi dan keamanan

- API key disimpan di **Windows Credential Manager**, bukan di file biasa.
- Audio disimpan sebagai file baru di folder hasil (bawaannya `D:\VoiceKit`, bisa diganti).
- Aplikasi hanya terhubung ke `api.elevenlabs.io` dan tidak mengirim data ke tempat lain.

## Butuh internet dan kuota

Pembuatan suara dilakukan di server ElevenLabs, jadi butuh internet dan memakai kuota karakter akunmu. Paket gratis ElevenLabs sudah bisa memakai v4. Voice cloning butuh paket yang mendukungnya.

## Menjalankan dari kode

Butuh [Rust](https://rustup.rs), [Node.js](https://nodejs.org), dan di Windows: Visual Studio Build Tools (workload C++).

```bash
npm install
npm run dev                         # mode pengembangan
npm run build                       # installer di src-tauri/target/release/bundle/
cargo test --manifest-path src-tauri/Cargo.toml
```

Untuk pengujian, `VOICEKIT_API_BASE` bisa diarahkan ke server tiruan.

## Lisensi

MIT. VoiceKit tidak berafiliasi dengan ElevenLabs. Pemakaian suara dan hasil audio mengikuti ketentuan layanan ElevenLabs dan paket akunmu.
