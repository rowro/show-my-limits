#!/bin/sh
# Invoked by sudo, which reads stdout directly. Never log or store the password.
if [ ! -x /usr/bin/zenity ]; then
    printf '%s\n' 'SML_ERROR:askpass_missing' >&2
    exit 1
fi
exec /usr/bin/zenity --password \
    --title='Show My Limits — разрешение перехвата' \
    --text='sudo запрашивает пароль для запуска сетевого компонента mitmproxy. Пароль не сохраняется.'
