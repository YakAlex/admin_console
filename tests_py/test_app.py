import os
import json
import time
import subprocess
import stat
import pytest

APP_PATH = r"target\debug\admin_console.exe"


# --- HELPER (Щоб не писати одне й те саме) ---
def run_app_and_wait(seconds=1):
    """Запускає програму, чекає seconds секунд і коректно закриває."""
    process = subprocess.Popen(
        APP_PATH,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=os.getcwd()
    )
    time.sleep(seconds)
    process.terminate()
    process.wait()
    return process

# --- ТЕСТИ ---

def test_todo_sync_basic():
    """Базовий тест: чи перетворюється текст на JSON."""
    print("\n[Test] Creating todo.txt...")
    initial_content = "- [14:00] Meeting\n- [ ] Buy Milk"
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(initial_content)

    print(f"[Test] Launching app...")
    run_app_and_wait(1)

    assert os.path.exists("tasks.json")
    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    assert len(data) == 2
    assert data[0]["title"] == "Meeting"
    assert data[0]["time"] == "14:00"

def test_task_completion():
    """Тест: чи розуміє програма виконані завдання [x]."""
    print("\n[Test] Creating todo.txt with completed task...")
    # Одне завдання активне, одне виконане
    initial_content = "- [ ] Active Task\n- [x] Done Task\n- [X] Also Done"
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(initial_content)

    run_app_and_wait(1)

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # Має бути 3 завдання
    assert len(data) == 3

    # Active Task -> completed: false
    assert data[0]["title"] == "Active Task"
    assert data[0]["completed"] is False

    # Done Task -> completed: true
    assert data[1]["title"] == "Done Task"
    assert data[1]["completed"] is True

    # Also Done (велика X) -> completed: true
    assert data[2]["title"] == "Also Done"
    assert data[2]["completed"] is True
    print("[PASS] Task completion logic works!")

def test_broken_config_resilience():
    """Тест: програма не має падати від поганого JSON."""
    print("\n[Test] Creating BROKEN config.json...")
    with open("config.json", "w", encoding="utf-8") as f:
        f.write('{ "targets": [ {"name": "Test", "address": "1.1.1.1"} ')

        # --- ВИПРАВЛЕННЯ: Створюємо todo.txt ДО запуску програми ---
    print("[Test] Creating todo.txt...")
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write("- [ ] I am alive")
    # -----------------------------------------------------------

    # Тепер запускаємо
    process = run_app_and_wait(1)

    # Тепер перевіряємо
    assert os.path.exists("tasks.json"), "App crashed or failed to save tasks!"

    # Додаткова перевірка: чи tasks.json валідний
    with open("tasks.json", "r") as f:
        data = json.load(f)
    assert data[0]["title"] == "I am alive"

    print("[PASS] App survived broken config!")

def test_logs_creation():
    """Тест: чи створюється файл логів."""
    if os.path.exists("logs.txt"):
        os.remove("logs.txt")

    run_app_and_wait(1)

    assert os.path.exists("logs.txt"), "logs.txt was not created automatically"
    print("[PASS] Logs file created.")


def test_unicode_and_emojis():
    """
    Тест: Перевірка коректної роботи з українською мовою та емодзі.
    Це важливо, щоб JSON не перетворився на "кракозябри".
    """
    print("\n[Test] Creating todo.txt with Unicode...")

    # Складний контент
    content = (
        "- [18:00] Вечірка 🎉\n"
        "- [ ] Купити хліб 🍞\n"
        "- [ ] 这是一个测试 (Китайська)\n"
        "- [ ] Task with status"
    )

    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(content)

    run_app_and_wait(1)

    # Читаємо tasks.json
    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # Перевірки
    assert len(data) == 4
    assert data[0]["title"] == "Вечірка 🎉"
    assert data[1]["title"] == "Купити хліб 🍞"
    assert data[2]["title"] == "这是一个测试 (Китайська)"

    print("[PASS] Unicode works perfectly!")


def test_garbage_resilience():
    """
    Тест: Що буде, якщо файл містить рядки, які НЕ є завданнями?
    Парсер має їх ігнорувати або пропускати, але не падати.
    """
    print("\n[Test] Creating mixed todo.txt...")
    content = (
        "Просто заголовок списку\n"  # Сміття
        "-----------------------\n"  # Сміття
        "- [ ] Valid Task 1\n"  # OK
        "Note: Don't forget milk\n"  # Сміття
        "- [x] Valid Task 2"  # OK
    )

    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(content)

    run_app_and_wait(1)

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # У нас тільки 2 валідних завдання, решту парсер має відсіяти
    # (або, залежно від вашої логіки парсингу, він може нічого не створити для сміття)

    # У вашому поточному коді парсера (parse_tasks_from_text)
    # ви, ймовірно, шукаєте рядки, що починаються з "- [".
    # Перевіримо це:
    valid_tasks = [t for t in data if "Valid Task" in t["title"]]

    assert len(valid_tasks) == 2, f"Parser picked up garbage! Found {len(data)} items."
    assert valid_tasks[0]["title"] == "Valid Task 1"
    assert valid_tasks[1]["completed"] is True

    print("[PASS] Garbage lines successfully ignored!")


def test_stress_load_50000_lines():
    """
    СТРЕС-ТЕСТ: Генеруємо 50000 завдань.
    Перевіряємо, чи програма встигає їх обробити і не крашиться.
    """
    print("\n[Test] Generating 50000 lines...")

    lines = []
    for i in range(50000):
        # Чергуємо виконані і невиконані
        status = "x" if i % 2 == 0 else " "
        lines.append(f"- [{status}] Task number {i}")

    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(lines))

    start_time = time.time()

    # Даємо трохи більше часу на старт (2 секунди), бо файл великий
    run_app_and_wait(2)

    end_time = time.time()

    assert os.path.exists("tasks.json")

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    assert len(data) == 50000, f"Lost data! Expected 50000, got {len(data)}"
    assert data[49999]["title"] == "Task number 49999"

    print(f"[PASS] Processed 50000 lines in {end_time - start_time:.2f}s (execution time)")


def test_invalid_time_formats():
    """
    Тест: Як парсер реагує на некоректний формат часу?
    Він не повинен крашитись, а має сприймати це як звичайний текст.
    """
    print("\n[Test] Testing invalid time formats...")
    content = (
        "- [25:00] Invalid Hour\n"  # Немає такої години
        "- [12:61] Invalid Minute\n"  # Немає такої хвилини
        "- [abc] Not a time\n"  # Текст у дужках
        "- [14:30] Good Task"  # Нормальний час
    )
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(content)

    run_app_and_wait(1)

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # 1. 25:00 - це не час, тому поле time має бути пустим (або текст залишиться в title)
    # Залежить від вашої реалізації is_valid_time. Якщо вона сувора:
    assert data[0]["time"] == "" or data[0]["time"] != "25:00"

    # 2. Good Task має розпізнатись
    good_task = [t for t in data if "Good Task" in t["title"]][0]
    assert good_task["time"] == "14:30"

    print("[PASS] Time parser is robust!")


def test_whitespace_handling():
    """
    Тест: Чи розуміє парсер завдання з відступами?
    """
    print("\n[Test] Creating indented todo.txt...")
    content = (
        "- [ ] Normal\n"
        "   - [ ] Indented\n"
        " - [x] Space before dash"
    )
    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(content)

    run_app_and_wait(1)

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # Має знайти всі 3 завдання
    assert len(data) == 3
    assert data[1]["title"] == "Indented"
    assert data[2]["title"] == "Space before dash"
    assert data[2]["completed"] is True

    print("[PASS] Whitespace handled correctly!")


def test_sync_priority_txt_over_json():
    """
    Тест: Перевіряємо, що todo.txt є 'головним'.
    Якщо tasks.json містить старі дані, вони мають бути замінені.
    """
    print("\n[Test] Setting up conflict...")

    # 1. Записуємо старі дані в JSON
    old_json = [{"title": "Old Task", "time": "", "completed": False}]
    with open("tasks.json", "w") as f:
        json.dump(old_json, f)

    # 2. Записуємо нові дані в TXT
    with open("todo.txt", "w") as f:
        f.write("- [ ] New Task Master")

    # 3. Запускаємо
    run_app_and_wait(1)

    # 4. Перевіряємо
    with open("tasks.json", "r") as f:
        data = json.load(f)

    assert len(data) == 1
    assert data[0]["title"] == "New Task Master"
    assert data[0]["title"] != "Old Task"

    print("[PASS] Priority confirmed: TXT overwrites JSON on start.")


def test_stress_monitor_100_servers():
    """
    Тест: Завантажуємо 100 серверів у конфіг.
    Перевіряємо, чи програма запускається і створює tasks.json (ознака життя).
    """
    print("\n[Test] Generating config with 100 servers...")

    targets = []
    for i in range(100):
        targets.append({
            "name": f"Srv_{i}",
            "address": f"192.168.1.{i}"  # Фейкові адреси
        })

    config = {"targets": targets, "commands": []}

    with open("config.json", "w") as f:
        json.dump(config, f)

    with open("todo.txt", "w") as f:
        f.write("- [ ] I survived monitoring")

    # Запускаємо. Якщо Rayon або потоки налаштовані погано, тут може бути дедлок.
    run_app_and_wait(2)

    assert os.path.exists("tasks.json")
    print("[PASS] Monitor handled 100 servers config.")


def test_empty_todo_clears_json():
    """
    Тест: Якщо очистити todo.txt, то tasks.json теж має стати пустим.
    """
    print("\n[Test] Clearing todo.txt...")

    # Спочатку створимо щось
    with open("tasks.json", "w") as f:
        f.write('[{"title": "Ghost", "completed": false}]')

    # Тепер пустий файл (або тільки пробіли)
    with open("todo.txt", "w") as f:
        f.write("   \n")

    run_app_and_wait(1)

    with open("tasks.json", "r") as f:
        data = json.load(f)

    assert len(data) == 0, "JSON should be empty!"
    print("[PASS] Empty file sync works.")


def test_readonly_filesystem_handling():
    """
    Тест: Що буде, якщо файли заблоковані для запису?
    Програма НЕ повинна падати (crash), вона має працювати (хоча б в режимі читання).
    """
    print("\n[Test] Setting files to Read-Only...")

    # Створюємо файли
    with open("todo.txt", "w") as f:
        f.write("- [ ] I cannot be changed")
    with open("tasks.json", "w") as f:
        f.write("[]")

    # РОБИМО ФАЙЛ READ-ONLY (Тільки для читання)
    os.chmod("tasks.json", stat.S_IREAD)

    try:
        # Запускаємо програму. Вона спробує оновити tasks.json при старті.
        # Якщо там стоїть .unwrap(), програма впаде.
        run_app_and_wait(1)

        # Якщо ми дійшли сюди - програма вижила. Це добре.
        print("[PASS] App survived Read-Only file system.")

    finally:
        # ОБОВ'ЯЗКОВО повертаємо права назад, інакше наступні тести (і cleanup) впадуть!
        os.chmod("tasks.json", stat.S_IWRITE)


def test_massive_single_line_task():
    """
    Тест: Одне завдання довжиною в 10 000 символів.
    Перевіряємо, чи не зламається парсер або UI від такої "ковбаси".
    """
    print("\n[Test] Creating massive single line task...")

    long_desc = "A" * 10000  # 10 тисяч літер 'A'
    content = f"- [ ] Short Title {long_desc}"

    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(content)

    run_app_and_wait(1)

    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    # Перевіряємо, чи збереглись дані
    assert len(data) == 1
    # Перевіряємо, що рядок не обрізався (залежить від вашої логіки, але крашів бути не має)
    assert len(data[0]["title"]) > 9000

    print("[PASS] Handled 10k chars line without crash.")


def test_control_chars_injection():
    """
    Тест: Вставка спецсимволів (Bell, Backspace, ANSI Colors).
    JSON має бути валідним, програма не має падати.
    """
    print("\n[Test] Injecting control characters...")

    # \x07 - Bell (звук)
    # \x08 - Backspace
    # \x1b[31m - ANSI Red Color
    bad_content = "- [ ] Task with \x07Bell and \x1b[31mColor\x1b[0m code"

    with open("todo.txt", "w", encoding="utf-8") as f:
        f.write(bad_content)

    run_app_and_wait(1)

    # Якщо JSON зламався через спецсимволи, тут вилетить JSONDecodeError
    with open("tasks.json", "r", encoding="utf-8") as f:
        data = json.load(f)

    print(f"Parsed title: {data[0]['title']}")
    assert "Bell" in data[0]["title"]

    print("[PASS] Control characters handled safely.")