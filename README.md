# rustpad

Notepad แบบ terminal เขียนด้วย Rust — Markdown + Vim keys + autosave + backup zip

## Build & run
    cargo build --release          # ต้องใช้ Rust 1.85+ (rustup update stable)
    ./target/release/rustpad       # เปิดโน้ตล่าสุด
    ./target/release/rustpad todo  # เปิด/สร้างโน้ต todo.md
    cargo install --path .         # ติดตั้งเป็นคำสั่ง `rustpad`

## ข้อมูลเก็บที่ไหน
`~/.local/share/rustpad/` (Linux) · `~/Library/Application Support/rustpad/` (macOS) · `%APPDATA%\rustpad\` (Windows)
เปลี่ยนได้ด้วย `RUSTPAD_HOME=/path`

    notes/    ไฟล์ .md จริงๆ (เปิดด้วยโปรแกรมอื่นได้)
    backups/  zip จาก :backup และ pre-restore-*.zip (สำรองอัตโนมัติก่อน restore)
    trash/    โน้ตที่ลบด้วย :rm!

## Backup / Restore
    :backup [file.zip]            หรือ  rustpad backup [file.zip]
    :restore <file.zip|latest>    หรือ  rustpad restore <file.zip|latest>
restore จะ **แทนที่** โน้ตทั้งหมดด้วยเนื้อหาใน zip (สถานะก่อนหน้าถูก zip เก็บไว้ให้เสมอ)

## คีย์ (กด F1 หรือ :help ในโปรแกรม)
Normal: h j k l w b e 0 ^ $ gg G {n}G { } · i a I A o O · x X D C s S r J · u Ctrl-r · p P
Operators: d y c + motion, dd yy cc, นับได้ (3dd, d2w, 5j) · Visual: v V แล้ว y d c · ค้นหา /pat n N
Ctrl-s บันทึก · Ctrl-p/F3 preview · F2 รายการโน้ต · Tab สลับ editor/list
Command: :w :q :q! :wq :new :open :rename :rm :rm! :backup :restore :preview :list :{บรรทัด}

## เมาส์
ไม่ได้ดักเมาส์ไว้ จึงลากเลือก/copy ด้วยเมาส์และ paste (Ctrl-Shift-V / คลิกกลาง) ได้ตามปกติของเทอร์มินัล
ถ้าอยากเลือกข้อความสะอาดๆ ให้ปิด list/preview ก่อน (F2, F3)
