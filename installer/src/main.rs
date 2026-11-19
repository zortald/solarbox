//! Solarbox installer: puts the server into a container on the site's MikroTik
//! router. Checks the router, shows what it will do and asks first. Every step
//! looks at the router's state before acting, so running it again after an
//! interruption (or after the router's reboots) continues where it stopped.
//! The router password is used for this run only and never stored.
//!
//!   solarbox-install [--router <ip>]
//! Unattended: --user <login> --yes, password in SOLARBOX_ROUTER_PASSWORD.
//! Test flags (a router behind port forwarding, e.g. CHR in QEMU):
//!   --rest-port <p> --ssh-port <p> --lan-ip <router's LAN address> --solarbox-url <url>

mod fetch;
mod ros;
mod ssh;

use ros::{s, Ros};
use serde_json::{json, Value};
use std::io::Write;
use std::time::Duration;

const MIN_ROUTEROS: (u32, u32) = (7, 25);
const FILE: &str = "solarbox.tar";
const NAME: &str = "solarbox";
const VETH: &str = "veth-sb";
const BRIDGE: &str = "cont-br";
const CT_IP: &str = "172.18.0.2";
const GW_IP: &str = "172.18.0.1";
/// RouterOS answers "too many unsuccessful attempts" from this count on.
const MAX_ATTEMPTS: u32 = 3;

fn step(m: &str) {
    println!("\n▸ {m}");
}
fn info(m: &str) {
    println!("  {m}");
}
fn fail(m: impl std::fmt::Display) -> ! {
    eprintln!("\n✗ {m}");
    std::process::exit(1)
}
fn ask(q: &str) -> String {
    print!("{q}");
    let _ = std::io::stdout().flush();
    let mut l = String::new();
    if std::io::stdin().read_line(&mut l).unwrap_or(0) == 0 {
        fail("ввод закрыт");
    }
    l.trim().to_string()
}
fn yes(q: &str) -> bool {
    matches!(ask(&format!("{q} [д/н]: ")).to_lowercase().as_str(), "д" | "да" | "y" | "yes")
}
fn ok<T>(r: Result<T, impl std::fmt::Display>, what: &str) -> T {
    r.unwrap_or_else(|e| fail(format!("{what}: {e}")))
}

struct Args {
    router: Option<String>,
    rest_port: u16,
    ssh_port: u16,
    lan_ip: Option<String>,
    url: Option<String>,
    user: Option<String>,
    yes: bool,
}

fn args() -> Args {
    let mut a = Args { router: None, rest_port: 80, ssh_port: 22, lan_ip: None, url: None, user: None, yes: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| fail(format!("{k}: нужно значение")));
        match k.as_str() {
            "--router" => a.router = Some(v()),
            "--rest-port" => a.rest_port = v().parse().unwrap_or_else(|_| fail("--rest-port: число")),
            "--ssh-port" => a.ssh_port = v().parse().unwrap_or_else(|_| fail("--ssh-port: число")),
            "--lan-ip" => a.lan_ip = Some(v()),
            "--solarbox-url" => a.url = Some(v()),
            "--user" => a.user = Some(v()),
            "--yes" => a.yes = true,
            "-h" | "--help" => {
                println!("solarbox-install [--router <адрес роутера>]");
                std::process::exit(0)
            }
            _ => fail(format!("неизвестный параметр {k}")),
        }
    }
    a
}

/// This computer's default gateway — normally the site router.
fn gateway() -> Option<String> {
    netdev::get_default_gateway().ok()?.ipv4.first().map(|ip| ip.to_string())
}

fn by_name(ros: &Ros, path: &str, name: &str) -> Option<Value> {
    ros.list(&format!("{path}?name={name}")).ok()?.into_iter().next()
}

/// What is already on the router, read before planning.
struct State {
    package: bool,
    device_mode: bool,
    attempts: u32,
    container: Option<Value>,
}

fn read_state(ros: &Ros) -> State {
    let pkg = by_name(ros, "/system/package", "container");
    let dm = ok(ros.get("/system/device-mode"), "режим устройства");
    State {
        package: pkg.as_ref().is_some_and(|p| s(p, "disabled") == "false" && !s(p, "version").is_empty()),
        device_mode: s(&dm, "container") == "true",
        attempts: s(&dm, "attempt-count").parse().unwrap_or(0),
        container: by_name(ros, "/container", NAME),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let a = args();
    println!("Solarbox — установка на роутер MikroTik");
    println!("Пароль роутера нужен только на время установки и нигде не сохраняется.");

    step("Роутер");
    let host = a.router.clone().or_else(gateway).unwrap_or_default();
    let host = match a.router.is_some() || a.yes {
        true => host,
        false => match ask(&format!("  Адрес роутера [{host}]: ")) {
            x if x.is_empty() => host,
            x => x,
        },
    };
    if host.is_empty() {
        fail("не знаю адрес роутера — укажите его");
    }
    let user = a.user.clone().unwrap_or_else(|| match ask("  Логин [admin]: ") {
        x if x.is_empty() => "admin".to_string(),
        x => x,
    });
    let pw = std::env::var("SOLARBOX_ROUTER_PASSWORD").unwrap_or_else(|_| rpassword::prompt_password("  Пароль: ").unwrap_or_else(|e| fail(e)));
    let mut ros = Ros::new(&format!("{host}:{}", a.rest_port), &user, &pw);

    let res = match ros.get("/system/resource") {
        Ok(v) => v,
        Err(ros::Err::Down(e)) => fail(format!(
            "{host} не отвечает по REST ({e}).\n  Это точно MikroTik? Нужна служба www: IP → Services → www (порт 80) включена и доступна из вашей сети."
        )),
        Err(e) => fail(e),
    };
    let (version, arch_name) = (s(&res, "version").to_string(), s(&res, "architecture-name").to_string());
    info(&format!("{} · RouterOS {version} · {arch_name}", s(&res, "board-name")));

    // ---- can it run Solarbox at all ----
    let arch = ros::server_arch(&arch_name).unwrap_or_else(|| {
        fail(format!("у процессора этого роутера ({arch_name}) нет поддержки контейнеров в RouterOS — сюда Solarbox не поставить.\n  Можно поставить его на любой компьютер с Linux (Raspberry Pi, NAS) через Docker."))
    });
    if !ros::version_at_least(&version, MIN_ROUTEROS) {
        fail(format!("нужен RouterOS {}.{} или новее (сейчас {version}). Обновите: System → Packages → Check For Updates → Download&Install, потом запустите установку снова.", MIN_ROUTEROS.0, MIN_ROUTEROS.1));
    }
    let mb = |k: &str| s(&res, k).parse::<u64>().unwrap_or(0) / 1_000_000;
    if mb("free-hdd-space") < 30 {
        fail(format!("мало места на роутере: свободно {} МБ, нужно ≈30", mb("free-hdd-space")));
    }
    if mb("free-memory") < 40 {
        fail(format!("мало свободной памяти на роутере: {} МБ, нужно ≈40", mb("free-memory")));
    }

    let lan = a.lan_ip.clone().unwrap_or_else(|| host.clone());
    if !ros.list("/ip/address").unwrap_or_default().iter().any(|x| s(x, "address").split('/').next() == Some(lan.as_str())) {
        fail(format!("у роутера нет адреса {lan} — запустите установку с компьютера в сети этого роутера (или укажите --router <его адрес в сети>)"));
    }

    let st = read_state(&ros);
    if let Some(c) = &st.container {
        info(&format!("Solarbox уже стоит на этом роутере (контейнер «{NAME}», {}).", if s(c, "running") == "true" { "работает" } else { "остановлен" }));
        info("Обновляется он сам: Настройки → Обновить. Здесь делать нечего.");
        return;
    }

    // ---- an empty admin password: SSH would only ask to change it ----
    if pw.is_empty() {
        if a.yes {
            fail("у роутера пустой пароль — задайте его (System → Users) и запустите снова");
        }
        step("Пароль роутера");
        info("У роутера пустой пароль. Так его может настроить любой в сети, и установка через SSH не пройдёт.");
        let p1 = rpassword::prompt_password("  Новый пароль роутера: ").unwrap_or_else(|e| fail(e));
        let p2 = rpassword::prompt_password("  Ещё раз: ").unwrap_or_else(|e| fail(e));
        if p1.len() < 8 || p1 != p2 {
            fail("пароли не совпадают или короче 8 символов — роутер не трогал");
        }
        ok(ros.post("/user/set", json!({ "numbers": user, "password": p1 })), "смена пароля");
        ros.pw = p1;
        info("Пароль задан. Запишите его — это пароль самого роутера.");
    }

    // ---- the plan ----
    step("Что будет сделано");
    let mut n = 0;
    let mut item = |t: &str| {
        n += 1;
        info(&format!("{n}. {t}"));
    };
    if !st.package {
        item("Поставить пакет container (роутер скачает его у MikroTik и перезагрузится, ≈1–2 мин).");
    }
    if !st.device_mode {
        item("Включить контейнеры. RouterOS требует подтверждения руками: нужно будет выключить питание роутера секунд на 10 (или коротко нажать Reset). Роутер перезагрузится.");
    }
    item(&format!("Скачать Solarbox под {arch}, проверить подпись и загрузить на роутер (≈4 МБ)."));
    item(&format!("Сеть для контейнера: {VETH} {CT_IP}, мост {BRIDGE} {GW_IP}; порт 8080 на {lan} → Solarbox."));
    item("Правило фаервола: контейнеру можно обращаться к REST роутера (порт 80), больше никуда.");
    item(&format!("Контейнер «{NAME}» с данными в /sb-data, запуск при загрузке роутера."));
    println!();
    info("Ваши настройки, правила и пользователи роутера не меняются, только добавляется перечисленное.");
    if !a.yes && !yes("\n  Ставить?") {
        println!("Ничего не сделано.");
        return;
    }

    // ---- 1. container package ----
    if !st.package {
        step("Пакет container");
        ok(ros.post("/system/package/update/check-for-updates", json!({ "duration": "10s" })), "проверка пакетов");
        ok(ros.post("/system/package/enable", json!({ "numbers": "container" })), "включение пакета");
        info("Роутер скачивает пакет и перезагружается…");
        let _ = ros.post("/system/package/apply-changes", json!({}));
        tokio::time::sleep(Duration::from_secs(20)).await;
        if !ros.wait_up(600) || !read_state(&ros).package {
            fail("пакет container не появился после перезагрузки. Проверьте интернет на роутере и запустите установку снова — она продолжит.");
        }
        info("Готово.");
    }

    // ---- 2. device-mode: the physical confirmation ----
    if !read_state(&ros).device_mode {
        step("Включение контейнеров");
        let attempts = read_state(&ros).attempts;
        if attempts >= MAX_ATTEMPTS {
            info("RouterOS насчитал слишком много неподтверждённых попыток. Сейчас начнётся ещё одна — подтвердите её питанием, счётчик сбросится, затем запустите установку снова.");
        }
        let sshc = ok(ssh::Ssh::connect(&host, a.ssh_port, &ros.user, &ros.pw).await, "SSH");
        println!();
        info("Сейчас роутер ждёт подтверждения от человека рядом с ним:");
        info("  выключите питание роутера на 10 секунд и включите снова");
        info("  (или коротко нажмите кнопку Reset — НЕ удерживайте её, долгое нажатие сбрасывает настройки).");
        info("Если роутер питается и от адаптера, и по PoE — отключите оба. На это есть 30 минут.");
        println!();
        let mut last_min = String::new();
        let r = sshc
            .run_until_closed("/system/device-mode/update container=yes activation-timeout=30m", |l| {
                if let Some(t) = l.split("turn off power in ").nth(1) {
                    let t = t.split_whitespace().next().unwrap_or("");
                    let min = t.split('m').next().unwrap_or("").to_string();
                    if t.contains('m') && min != last_min {
                        info(&format!("ждёт подтверждения питанием, осталось {t}"));
                        last_min = min;
                    }
                } else if !l.starts_with('[') {
                    info(&format!("роутер: {l}"));
                }
            })
            .await;
        drop(sshc);
        if let Err(e) = r {
            info(&format!("сессия закрылась: {e}"));
        }
        info("Ждём, пока роутер снова загрузится…");
        tokio::time::sleep(Duration::from_secs(10)).await;
        if !ros.wait_up(600) {
            fail("роутер не отвечает 10 минут после перезагрузки — проверьте его и запустите установку снова");
        }
        let st = read_state(&ros);
        if !st.device_mode {
            fail(format!(
                "контейнеры не включились (попыток без подтверждения: {}). Скорее всего, питание не выключали или нажали не ту кнопку. Запустите установку снова.",
                st.attempts
            ));
        }
        info("Контейнеры включены.");
    }

    // ---- 3. image ----
    step("Solarbox");
    let (ver, image) = ok(fetch::routeros_image(arch, info), "образ");
    info(&format!("версия {ver}, {} КБ, подпись верна", image.len() / 1024));
    let sshc = ok(ssh::Ssh::connect(&host, a.ssh_port, &ros.user, &ros.pw).await, "SSH");
    ok(sshc.upload(FILE, &image).await, "загрузка на роутер");
    drop(sshc);
    info("загружен на роутер");

    // ---- 4. network and firewall ----
    step("Сеть");
    ok(ros.ensure("/interface/veth", &format!("?name={VETH}"), json!({ "name": VETH, "address": format!("{CT_IP}/24"), "gateway": GW_IP })), "veth");
    ok(ros.ensure("/interface/bridge", &format!("?name={BRIDGE}"), json!({ "name": BRIDGE })), "мост");
    ok(ros.ensure("/ip/address", &format!("?address={GW_IP}/24"), json!({ "address": format!("{GW_IP}/24"), "interface": BRIDGE })), "адрес моста");
    ok(ros.ensure("/interface/bridge/port", &format!("?interface={VETH}"), json!({ "bridge": BRIDGE, "interface": VETH })), "порт моста");
    ok(
        ros.ensure(
            "/ip/firewall/nat",
            "?comment=solarbox",
            json!({ "chain": "dstnat", "action": "dst-nat", "protocol": "tcp", "dst-address": lan, "dst-port": "8080",
                    "to-addresses": CT_IP, "to-ports": "8080", "comment": "solarbox" }),
        ),
        "проброс порта 8080",
    );
    let rule = "solarbox: REST роутера";
    if ok(ros.list("/ip/firewall/filter"), "фаервол").iter().all(|r| s(r, "comment") != rule) {
        let mut r = json!({ "chain": "input", "action": "accept", "protocol": "tcp", "src-address": CT_IP,
                            "dst-address": GW_IP, "dst-port": "80", "comment": rule });
        // First in the list: the default input chain drops what isn't from the LAN.
        if let Some(first) = ros.list("/ip/firewall/filter").unwrap_or_default().first() {
            r["place-before"] = json!(s(first, ".id"));
        }
        ok(ros.add("/ip/firewall/filter", r), "правило фаервола");
    }
    info("готово");

    // ---- 5. container ----
    step("Контейнер");
    ok(ros.ensure("/container/mounts", "?list=sb-data", json!({ "list": "sb-data", "src": "/sb-data", "dst": "/data" })), "папка данных");
    let mut c = json!({ "file": FILE, "interface": VETH, "root-dir": "sb-root", "mountlists": "sb-data",
                        "logging": "yes", "start-on-boot": "yes", "name": NAME, "restart-policy": "unless-stopped" });
    if ros.add("/container", c.clone()).is_err() {
        // Older 7.x without restart policies.
        c.as_object_mut().unwrap().remove("restart-policy");
        ok(ros.add("/container", c), "создание контейнера");
    }
    info("распаковывается…");
    let mut id = String::new();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Some(c) = by_name(&ros, "/container", NAME) {
            id = s(&c, ".id").to_string();
            if s(&c, "stopped") == "true" || s(&c, "running") == "true" {
                break;
            }
        }
    }
    ok(ros.post("/container/start", json!({ ".id": id })), "запуск");
    let _ = ros.post("/file/remove", json!({ "numbers": FILE }));

    let url = a.url.clone().unwrap_or_else(|| format!("http://{lan}:8080"));
    let up = (0..40).any(|_| {
        std::thread::sleep(Duration::from_secs(3));
        ureq::get(&format!("{url}/healthz")).timeout(Duration::from_secs(3)).call().is_ok()
    });
    if !up {
        fail(format!("Solarbox не отвечает на {url} — посмотрите журнал роутера (Log), темы container"));
    }

    // ---- done ----
    let log = ros.list("/log").unwrap_or_default();
    let find = |key: &str| log.iter().rev().find_map(|m| s(m, "message").split(key).nth(1).map(|x| x.trim().to_string()));
    step("Готово");
    info(&format!("Solarbox работает: https://{lan}:8080"));
    info("Браузер предупредит о сертификате — он свой, самоподписанный. Сверьте отпечаток и подтвердите:");
    if let Some(fp) = find("certificate SHA-256") {
        info(&format!("  {fp}"));
    }
    info("Входите логином и паролем роутера. Дальше Solarbox сам проведёт по настройке.");
    if let Some(p) = find("(empty user name, also in /data/admin-password):") {
        println!();
        info(&format!("Аварийный пароль Solarbox (вход с пустым логином): {p}"));
        info("Запишите его: он нужен, если роутер недоступен.");
    }
}
