// /phone on this PC: turn chat access from the local network on/off and show the QR code a
// phone scans to sign in. The core sends this status to this PC's UI only (it has the token).

import { useState } from "react";
import { Copy, RefreshCw, Smartphone } from "lucide-react";
import type { Connection } from "../api/connection";
import { useUiState } from "../api/hooks";

export function PhonePage({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const lan = ui.lan;
  const [picked, setPicked] = useState(0);
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

  const address = lan.addresses[Math.min(picked, lan.addresses.length - 1)] ?? null;
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
        Телефон в той же сети Wi-Fi, что и этот ПК, открывает тот же чат: вопросы, история, кадры игры. 📷 с
        телефона снимает игру, которая сейчас на экране ПК.
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
            {lan.addresses.length > 1 && (
              <label className="phone-adapter">
                Сеть:{" "}
                <select value={picked} onChange={(e) => setPicked(Number(e.target.value))}>
                  {lan.addresses.map((a, i) => (
                    <option key={a.url} value={i}>
                      {a.adapter} — {new URL(a.url).hostname}
                    </option>
                  ))}
                </select>
              </label>
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
