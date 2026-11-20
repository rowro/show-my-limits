use std::{io::Read, sync::{Arc, Mutex}};

pub fn capture_diagnostic(line: &str) -> Option<&'static str> {
    let line = line.to_lowercase();
    if line.contains("sml_error:state_write") {
        return Some(
            "Перехватчик не может записать состояние в каталог приложения. Проверьте владельца и права каталога данных Show My Limits.",
        );
    }
    if line.contains("error in script") || line.contains("no module named") {
        return Some(
            "Не удалось загрузить обработчик лимитов. Переустановите Show My Limits и повторите запуск.",
        );
    }
    if line.contains("sml_error:askpass_missing") {
        return Some(
            "Не найден zenity для графического запроса прав. Установите его: sudo apt install zenity.",
        );
    }
    if [
        "failed to elevate privileges",
        "not running as root",
        "a terminal is required",
        "a password is required",
        "no tty present",
    ]
    .iter()
    .any(|message| line.contains(message))
    {
        Some(
            "Не удалось получить права администратора для перехвата Linux. Установка сертификата не выдаёт эти права. Подтвердите запрос пароля для сетевого компонента или проверьте возможность использования sudo.",
        )
    } else if line.contains("address already in use") {
        Some("Не удалось запустить перехватчик: сетевой адрес уже занят другим процессом.")
    } else if line.contains("operation not permitted") || line.contains("permission denied") {
        Some(
            "ОС запретила запуск сетевого перехвата. Проверьте разрешения; установка сертификата сама по себе их не выдаёт.",
        )
    } else {
        None
    }
}

pub fn monitor_capture_output(
    mut pipe: impl Read + Send + 'static,
    issue: Arc<Mutex<Option<String>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 1024];
        let mut line = Vec::with_capacity(4096);
        loop {
            let count = match pipe.read(&mut buffer) {
                Ok(count) => count,
                Err(_) => break,
            };
            for byte in buffer[..count]
                .iter()
                .copied()
                .chain((count == 0).then_some(b'\n'))
            {
                if byte == b'\n' {
                    if let Some(category) = capture_diagnostic(&String::from_utf8_lossy(&line)) {
                        *issue.lock().unwrap_or_else(|p| p.into_inner()) = Some(category.into());
                    }
                    line.clear();
                } else if line.len() < 4096 {
                    line.push(byte);
                }
            }
            if count == 0 {
                break;
            }
        }
        // Only fixed diagnostic categories survive; never persist raw helper output.
    })
}

