import pytest
import os
import time

# Список файлів, які створює ваша програма
FILES_TO_CLEAN = ["todo.txt", "tasks.json", "notes.txt", "logs.txt", "config.json"]


@pytest.fixture(autouse=True)
def clean_environment():
    """
    Ця функція запускається автоматично перед кожним тестом.
    Вона видаляє старі файли, щоб тест проходив у чистих умовах.
    """
    # 1. SETUP (Перед тестом)
    # Піднімаємось на рівень вище, в корінь Rust-проєкту
    os.chdir(os.path.join(os.path.dirname(__file__), ".."))

    _remove_files()

    yield  # Тут виконується сам тест

    # 2. TEARDOWN (Після тесту)
    # Можна розкоментувати, якщо хочете видаляти файли після тесту.
    # Але краще залишити, щоб подивитися результат очима.
    # _remove_files()


def _remove_files():
    for file in FILES_TO_CLEAN:
        if os.path.exists(file):
            try:
                os.remove(file)
            except PermissionError:
                pass  # Якщо файл зайнятий, пропускаємо