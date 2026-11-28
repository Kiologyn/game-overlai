// /phone on this PC: turn chat access from the local network on/off and show the QR code a
// phone scans to sign in. The core sends this status to this PC's UI only (it has the token).

import { useState } from "react";
import { Copy, RefreshCw, Smartphone } from "lucide-react";
import type { Connection } from "../api/connection";
import { useUiState } from "../api/hooks";
import { useT } from "../i18n";

export function PhonePage({ conn }: { conn: Connection }) {
  const ui = useUiState(conn);
  const t = useT();
  const lan = ui.lan;
  const [copied, setCopied] = useState(false);

  if (!lan) {
    return (
      <div className="phone-page">
        <h1>{t("phone")}</h1>
        <p className="phone-note">{ui.status === "open" ? t("phone_only_on_pc") : t("connecting_app")}</p>
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
        <h1>{t("phone_title")}</h1>
      </div>
      <p className="phone-note">{t("phone_intro")}</p>

      <label className="switch-row">
        <input
          type="checkbox"
          checked={lan.enabled}
          onChange={(e) => conn.send({ type: "lan.set", enabled: e.target.checked })}
        />
        <span className="switch" aria-hidden />
        <span>{lan.enabled ? t("lan_on") : t("lan_off")}</span>
      </label>

      {lan.error && <div className="phone-error">{lan.error}</div>}

      {running && lan.addresses.length === 0 && (
        <div className="phone-error">{t("lan_no_address")}</div>
      )}

      {running && address && (
        <div className="phone-card">
          <div className="phone-qr" dangerouslySetInnerHTML={{ __html: address.qrSvg }} />
          <div className="phone-card-text">
            <p>{t("phone_scan")}</p>
            <div className="phone-url">
              <code>{address.url.replace(/\?t=.*/, "")}</code>
              <button type="button" className="btn" onClick={copy}>
                <Copy size={13} aria-hidden /> {copied ? t("copied") : t("copy_link")}
              </button>
            </div>
            <label className="phone-adapter">
              <span>{t("network_interface")}</span>
              <select value={chosenAddress ? chosenAddress.adapter : ""} onChange={(e) => choose(e.target.value || null)}>
                <option value="">
                  {t("interface_auto", { adapter: lan.addresses[0].adapter, ip: lan.addresses[0].ip })}
                </option>
                {recommended.length > 0 && (
                  <optgroup label={t("interfaces_local")}>
                    {recommended.map((a) => (
                      <option key={a.adapter + a.ip} value={a.adapter}>
                        {a.adapter} — {a.ip}
                      </option>
                    ))}
                  </optgroup>
                )}
                {others.length > 0 && (
                  <optgroup label={t("interfaces_other")}>
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
              <p className="phone-warn">{t("interface_missing", { adapter: chosen })}</p>
            )}
            {!address.recommended && (
              <p className="phone-warn">{t("interface_not_local")}</p>
            )}
            <p className="phone-clients">
              {lan.clients === 0 ? t("devices_none") : t("devices_connected", { count: lan.clients })}
            </p>
            <button
              type="button"
              className="btn"
              onClick={() => {
                if (confirm(t("reset_access_confirm"))) {
                  conn.send({ type: "lan.resetToken" });
                }
              }}
            >
              <RefreshCw size={13} aria-hidden /> {t("reset_access")}
            </button>
          </div>
        </div>
      )}

      <ul className="phone-hints">
        <li>{t("phone_hint_firewall")}</li>
        <li>{t("phone_hint_private")}</li>
        <li>{t("phone_hint_unencrypted")}</li>
      </ul>
    </div>
  );
}
