// /phone on this PC: turn chat access from the local network on/off and show the QR code a
// phone scans to sign in. The core sends this status to this PC's UI only (it has the token).

import { useState } from "react";
import { Copy, RefreshCw, Smartphone } from "lucide-react";
import type { Connection } from "../api/connection";
import { useUiState } from "../api/hooks";

export function PhonePage({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const lan = ui.lan;
  const [copied, setCopied] = useState(false);

  if (!lan) {
    return (
      <div className="phone-page">
        <h1>Телефон</h1>
        <p className="phone-note">
          {ui.status === "open"
            ? "Эта страница работает только на ПК, где запущен game-overlai."
            : "Подключение к приложению…"}
        </p>
      </div>
    );
  }

  // The chosen adapter, or the best guess (first) when automatic or the chosen one is gone.
  const chosen = lan.chosenAdapter;
  const chosenAddress = chosen === null ? undefined : lan.addresses.find((a) => a.adapter === chosen);
  const address = chosenAddress ?? lan.addresses[0] ?? null;
  const recommended = lan.addresses.filter((a) => a.recommended);
  const others = lan.addresses.filter((a) => !a.recommended);
  const choose = (adapter: string | null) => conn.send({ type: "lan.chooseAdapter", adapter });
  const running = lan.enabled && lan.port !== null;

  const copy = async () => {
    if (!address) return;
    try {
      await navigator.clipboard.writeText(address.url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard blocked: the link is selectable anyway */
    }
  };

  return (
    <div className="phone-page">
      <div className="phone-head">
        <Smartphone size={20} aria-hidden />
        <h1>Чат с телефона</h1>
      </div>
      <p className="phone-note">
        Телефон в той же сети Wi-Fi, что и этот ПК, открывает тот же чат: вопросы, история, кадры игры. Если вопрос
        про экран, агент сам снимет игру, которая сейчас на экране ПК.
      </p>

      <label className="switch-row">
        <input
          type="checkbox"
          checked={lan.enabled}
          onChange={(e) => conn.send({ type: "lan.set", enabled: e.target.checked })}
        />
        <span className="switch" aria-hidden />
        <span>Доступ из локальной сети {lan.enabled ? "включён" : "выключен"}</span>
      </label>

      {lan.error && <div className="phone-error">{lan.error}</div>}

      {running && lan.addresses.length === 0 && (
        <div className="phone-error">
          Не нашёл адрес ПК в локальной сети. Подключите ПК к роутеру (Wi-Fi или кабель) и обновите страницу.
        </div>
      )}

      {running && address && (
        <div className="phone-card">
          <div className="phone-qr" dangerouslySetInnerHTML={{ __html: address.qrSvg }} />
          <div className="phone-card-text">
            <p>Наведите камеру телефона на код и откройте ссылку. Телефон запомнит вход.</p>
            <div className="phone-url">
              <code>{address.url.replace(/\?t=.*/, "")}</code>
              <button type="button" className="btn" onClick={copy}>
                <Copy size={13} aria-hidden /> {copied ? "Скопировано" : "Ссылка со входом"}
              </button>
            </div>
            <label className="phone-adapter">
              <span>Сетевой интерфейс</span>
              <select value={chosenAddress ? chosenAddress.adapter : ""} onChange={(e) => choose(e.target.value || null)}>
                <option value="">
                  Автоматически ({lan.addresses[0].adapter}, {lan.addresses[0].ip})
                </option>
                {recommended.length > 0 && (
                  <optgroup label="Локальная сеть">
                    {recommended.map((a) => (
                      <option key={a.adapter + a.ip} value={a.adapter}>
                        {a.adapter} — {a.ip}
                      </option>
                    ))}
                  </optgroup>
                )}
                {others.length > 0 && (
                  <optgroup label="Виртуальные, VPN и другие">
                    {others.map((a) => (
                      <option key={a.adapter + a.ip} value={a.adapter}>
                        {a.adapter} — {a.ip}
                      </option>
                    ))}
                  </optgroup>
                )}
              </select>
            </label>
            {chosen !== null && !chosenAddress && (
              <p className="phone-warn">
                Выбранного интерфейса «{chosen}» сейчас нет (отключён или без адреса), код ведёт на автоматический.
              </p>
            )}
            {!address.recommended && (
              <p className="phone-warn">
                Это виртуальный, VPN или внешний адрес: телефон из домашней сети Wi-Fi, скорее всего, по нему не
                откроет чат.
              </p>
            )}
            <p className="phone-clients">
              {lan.clients === 0 ? "Устройств не подключено" : `Подключено устройств: ${lan.clients}`}
            </p>
            <button
              type="button"
              className="btn"
              onClick={() => {
                if (confirm("Все телефоны выйдут, и вход понадобится заново по новому коду. Сбросить?")) {
                  conn.send({ type: "lan.resetToken" });
                }
              }}
            >
              <RefreshCw size={13} aria-hidden /> Сбросить доступ
            </button>
          </div>
        </div>
      )}

      <ul className="phone-hints">
        <li>
          При первом включении Windows спросит, разрешить ли приложению доступ к сети: разрешите для частных сетей.
          В общественных сетях (кафе, отель) доступ лучше не включать.
        </li>
        <li>
          Если телефон не открывает страницу: проверьте, что он в той же сети, а сеть в Windows отмечена как
          частная (Параметры → Сеть и Интернет → Wi-Fi → свойства сети).
        </li>
        <li>Соединение не шифруется: включайте доступ только в своей домашней сети.</li>
      </ul>
    </div>
  );
}
