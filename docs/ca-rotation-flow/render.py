"""Render review mockups for the proposed CA rotation flow."""

from pathlib import Path
from xml.sax.saxutils import escape

OUT = Path(__file__).parent
W, H = 440, 650
ink, muted, green = "#26342d", "#748279", "#438366"
line, soft, amber = "#e5ebe4", "#e8f1e9", "#a76a2d"


def rect(x, y, w, h, fill, radius=0, stroke="none"):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{radius}" fill="{fill}" stroke="{stroke}"/>'


def text(x, y, value, size=14, color=ink, weight=400, anchor="start"):
    return (f'<text x="{x}" y="{y}" fill="{color}" font-size="{size}" '
            f'font-weight="{weight}" text-anchor="{anchor}" '
            f'font-family="Arial, DejaVu Sans, sans-serif">{escape(value)}</text>')


def lines(parts, x, y, size=13, color=muted, gap=20):
    return "".join(text(x, y + i * gap, value, size, color) for i, value in enumerate(parts))


def button(y, label, primary=True):
    fill = green if primary else "#eef1ec"
    color = "white" if primary else ink
    return rect(48, y, 344, 42, fill, 10) + text(220, y + 27, label, 14, color, 600, "middle")


def tag(x, y, label, fill=soft, color=green):
    width = max(72, len(label) * 8 + 18)
    return rect(x, y, width, 24, fill, 6) + text(x + 10, y + 16, label, 11, color, 600)


def frame(step, title, subtitle):
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">',
        rect(0, 0, W, H, "#eef1ec"),
        rect(20, 18, 400, 614, "white", 18, line),
        rect(42, 39, 29, 29, soft, 7),
        '<circle cx="56.5" cy="53.5" r="10" fill="none" stroke="#438366" stroke-width="4" stroke-dasharray="47 16" transform="rotate(-35 56.5 53.5)"/>',
        text(82, 58, "Show My Limits", 18, ink, 700),
        tag(335, 42, "МАКЕТ", "#f2f4f1", muted),
        rect(40, 84, 360, 1, line),
        text(48, 110, f"Настройки  /  Захват  /  Замена CA     {step} из 5", 12, muted),
        text(48, 150, title, 23, ink, 700),
        text(48, 176, subtitle, 13, muted),
        rect(40, 603, 360, 1, line),
        text(48, 622, "Выбранный сервер: api.anthropic.com", 11, muted),
    ]
    return parts


def card(y, height=100, fill="#f7f8f5"):
    return rect(40, y, 360, height, fill, 11, line)


screens = []

p = frame(1, "Старое доверие найдено", "Приложение пока работает с прежним CA.")
p += [card(198, 145), text(56, 226, "Действующий сертификат", 13, ink, 600),
      text(56, 253, "limits-mitmproxy", 16, ink, 600),
      text(56, 280, "SHA-256: 8A:72:…:E4", 12, muted),
      tag(56, 298, "System / Admin", "#fcf1df", amber),
      lines(["Старому корню доверяют все пользователи Mac.",
             "Замена начнётся только по вашему нажатию."], 48, 375),
      button(472, "Начать замену"),
      text(220, 545, "Старый CA и его ключ пока не изменятся", 12, muted, anchor="middle")]
screens.append(("01-old-trust", p))

p = frame(2, "Новый CA подготовлен", "Старый сертификат продолжает работать.")
p += [card(198, 139), text(56, 226, "Старый CA · активен", 13, muted, 600),
      text(56, 248, "SHA-256: 8A:72:…:E4", 12, muted),
      rect(56, 265, 328, 1, line),
      text(56, 288, "Новый CA · ещё не доверен", 13, ink, 600),
      text(56, 311, "SHA-256: 4F:19:…:91", 12, ink),
      lines(["Новый ключ создан в закрытом каталоге.",
             "Захват ещё использует старый CA."], 48, 369),
      button(471, "Доверять новому CA"),
      text(220, 544, "macOS покажет системное подтверждение", 12, muted, anchor="middle")]
screens.append(("02-new-ca", p))

p = frame(3, "Проверим новый CA", "Новое доверие подтверждено системой.")
p += [card(198, 73), text(56, 226, "Новый CA · текущий пользователь", 13, ink, 600),
      text(56, 250, "SHA-256: 4F:19:…:91", 12, muted),
      text(48, 308, "Для проверки:", 14, ink, 600),
      lines(["1. Запустить захват с новым CA", "2. Полностью перезапустить Claude",
             "3. Отправить сообщение и дождаться ответа"], 48, 339),
      button(455, "Проверить захват"),
      button(509, "Вернуться к старому CA", False)]
screens.append(("03-test-capture", p))

p = frame(4, "Новый CA работает", "Получен свежий ответ Claude через новый CA.")
p += [card(198, 82, soft), text(56, 229, "Новый CA активен", 15, green, 700),
      text(56, 254, "SHA-256: 4F:19:…:91", 12, ink),
      card(300, 135, "#fcf1df"),
      text(56, 329, "Осталось старое доверие", 15, amber, 700),
      text(56, 357, "System / Admin · 8A:72:…:E4", 12, ink),
      lines(["Пока оно есть, старый ключ остаётся риском.",
             "Удалим только сертификат с этим отпечатком."], 56, 384, 12, muted, 18),
      button(472, "Удалить старое доверие"),
      text(220, 545, "Можно отложить; предупреждение останется", 12, muted, anchor="middle")]
screens.append(("04-remove-old-trust", p))

p = frame(5, "Замена завершена", "Новый CA продолжает получать лимиты.")
p += [card(205, 187, soft), text(58, 237, "✓  Новый CA активен", 16, green, 700),
      text(58, 273, "✓  Старое доверие удалено", 15, ink, 600),
      text(58, 309, "✓  Старый закрытый ключ удалён", 15, ink, 600),
      text(58, 357, "Действующий SHA-256: 4F:19:…:91", 12, muted),
      lines(["Если системное удаление отменить, этот экран",
             "не появится: останется шаг очистки."], 48, 434),
      button(509, "Готово")]
screens.append(("05-complete", p))

for name, parts in screens:
    svg = OUT / f"{name}.svg"
    svg.write_text("".join(parts) + "</svg>", encoding="utf-8")
