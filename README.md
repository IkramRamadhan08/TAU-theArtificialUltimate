# Laundryku

Laundryku adalah aplikasi web manajemen laundry berbasis Laravel dan Filament.

## Login Demo

Setelah instalasi selesai, buka halaman admin:

```text
http://127.0.0.1:8000/admin
```

Gunakan akun berikut:

```text
Email: test@example.com
Password: password
```

## Instalasi Windows

Installer Windows menggunakan XAMPP untuk MySQL. Script akan membantu setup database, dependency, migration, seed user login, build asset, lalu menjalankan server Laravel.

1. Install XAMPP jika belum ada. Jika belum terdeteksi, script akan menawarkan instalasi via winget.
2. Buka PowerShell di folder project.
3. Jalankan:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-windows.ps1
```

Script akan menanyakan:

```text
XAMPP folder path
Laravel app port
Konfirmasi instalasi dependency yang belum ada
```

Default yang aman:

```text
XAMPP folder path: C:\xampp
Laravel app port: 8000
```

Catatan: project ini membutuhkan PHP 8.3 atau lebih baru. Jika PHP bawaan XAMPP masih di bawah 8.3, script tetap memakai MySQL dari XAMPP, tetapi akan menawarkan instalasi PHP 8.4 via winget untuk menjalankan Laravel.

## Instalasi Linux

Installer Linux akan mengecek dan memasang dependency yang belum tersedia, menyiapkan database lokal, menjalankan migration dan seeder, build asset, lalu menjalankan server Laravel.

Jalankan dari folder project:

```bash
chmod +x install-linux.sh
./install-linux.sh
```

Script mendukung package manager umum:

```text
apt
pacman
dnf
zypper
```

Database yang dipakai:

```text
SQLite jika ekstensi pdo_sqlite aktif
MariaDB lokal sebagai fallback jika SQLite tidak tersedia
```

## Setelah Server Jalan

Jika script selesai, terminal akan menampilkan:

```text
Laundryku is ready.
URL: http://127.0.0.1:8000/admin
Email: test@example.com
Password: password
```

Biarkan terminal tetap terbuka selama menggunakan aplikasi. Tekan `Ctrl+C` untuk menghentikan server.

## Menjalankan Ulang

Jika dependency sudah terpasang, cukup jalankan script yang sama:

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\install-windows.ps1
```

Linux:

```bash
./install-linux.sh
```

Migration dan seeder aman dijalankan ulang. Akun login demo akan tetap tersedia.
