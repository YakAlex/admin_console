import os
import json
import time
import subprocess
import pytest

# Шлях до exe
APP_PATH = r"target\debug\admin_console.exe"


def test_todo_sync():
    print("\n[Test] Creating todo.txt...")
    initial_content = "- [14:00] Meeting with Boss\n- [ ] Buy Milk"
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(initial_content)

    print(f"[Test] Launching {APP_PATH}...")

    # Запускаємо процес через subprocess
    # stdin=subprocess.PIPE дозволяє нам писати команди програмі
    # stdout=subprocess.PIPE дозволяє читати, що вона пише (щоб не смітила в консоль)
    process = subprocess.Popen(
        APP_PATH,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=os.getcwd()  # Запускаємо з поточної папки
    )

    print("[Test] App running. Waiting for processing...")
    # Даємо час на старт і читання файлів
    time.sleep(1)

    print("[Test] Sending Ctrl+Q equivalent...")
    # У звичайному режимі (не raw) crossterm може не ловити Ctrl+Q так само.
    # Але для тесту синхронізації нам достатньо просто вбити процес коректно або дати йому попрацювати.
    # Оскільки ми змінили main.rs, автозбереження спрацює через 30 секунд або при виході.
    # Ми просто примусово завершимо процес, але перед цим перевіримо, чи створився tasks.json
    # (Пам'ятаєте? Ми додали примусове оновлення tasks.json при старті)

    # М'яко просимо закритися (на Windows це terminate)
    process.terminate()
    process.wait()

    print("[Test] App closed. Checking files...")

    # Перевірка
    assert os.path.exists("tasks.json"), "tasks.json не був створений!"

    # Іноді файл може бути порожнім, якщо процес вбили занадто швидко під час запису
    # Тому читаємо обережно
    with open("tasks.json", "r", encoding="utf-8") as f:
        content = f.read()
        if not content:
            pytest.fail("tasks.json is empty!")
        data = json.loads(content)

    assert len(data) == 2, f"Expected 2 tasks, got {len(data)}"
    assert data[0]["title"] == "Meeting with Boss"
    assert data[0]["time"] == "14:00"

    print("[PASS] Sync logic works!")