//! The portal's pages: home (workspaces and report cards) and the report view (desktop, or the
//! phone layout in a phone frame).

use crate::server::Ctx;
use crate::store::Meta;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const LOGO: &str = r##"<svg viewBox="0 0 64 64" aria-hidden="true"><defs><linearGradient id="pl" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#3B6FE0"/><stop offset="1" stop-color="#14B8A6"/></linearGradient></defs><rect width="64" height="64" rx="14" fill="url(#pl)"/><g fill="#fff"><rect x="10" y="38" width="8" height="12" rx="2"/><rect x="22" y="26" width="8" height="24" rx="2"/><rect x="34" y="32" width="8" height="18" rx="2"/><rect x="46" y="16" width="8" height="34" rx="2"/></g></svg>"##;

const CSS: &str = r#"
*{box-sizing:border-box}html,body{margin:0;height:100%}
body{font-family:Inter,"Segoe UI",system-ui,-apple-system,Helvetica,Arial,sans-serif;color:#1f2430;background:#f4f6f9;-webkit-font-smoothing:antialiased}
a{color:inherit;text-decoration:none}
.top{height:52px;display:flex;align-items:center;gap:14px;padding:0 16px;background:#0f172a;color:#e2e8f0}
.top .logo{width:28px;height:28px}.top .logo svg{width:28px;height:28px;display:block}
.top .brand{font-weight:650;font-size:15px;white-space:nowrap}
.top .crumbs{display:flex;align-items:center;gap:8px;font-size:13.5px;color:#94a3b8;min-width:0}
.top .crumbs b{color:#fff;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.top .sp{flex:1}
.search{width:min(360px,40vw);height:32px;border:1px solid #334155;border-radius:7px;background:#1e293b;color:#e2e8f0;padding:0 12px;font:inherit;font-size:13px}
.search::placeholder{color:#94a3b8}
.wrap{display:flex;height:calc(100vh - 52px)}
.side{width:240px;flex:none;background:#fff;border-right:1px solid #e3e7ee;padding:14px 10px;overflow:auto}
.side h3{margin:16px 10px 6px;font-size:11px;letter-spacing:.05em;text-transform:uppercase;color:#667085;font-weight:600}
.side a{display:flex;align-items:center;justify-content:space-between;padding:8px 10px;border-radius:7px;font-size:13.5px;color:#344054}
.side a:hover{background:#f1f4f8}.side a.on{background:#e8f0fd;color:#1d4fb8;font-weight:600}
.side a span{font-size:11.5px;color:#98a2b3;font-weight:500}
.main{flex:1;overflow:auto;padding:28px 32px 48px}
h1{margin:0 0 4px;font-size:22px;font-weight:700;letter-spacing:-.01em}
.lead{margin:0 0 24px;color:#667085;font-size:13.5px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:20px}
.card{display:block;background:#fff;border:1px solid #e3e7ee;border-radius:12px;overflow:hidden;transition:box-shadow .15s,transform .15s}
.card:hover{box-shadow:0 10px 28px rgba(15,23,42,.10);transform:translateY(-1px)}
.thumb{aspect-ratio:16/9;background:#eef1f5;border-bottom:1px solid #eef0f3;display:flex;align-items:center;justify-content:center;overflow:hidden}
.thumb img{width:100%;height:100%;object-fit:contain;display:block}
.meta{padding:12px 14px 14px}
.meta .t{font-weight:600;font-size:14.5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.meta .s{margin-top:3px;font-size:12px;color:#667085;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.tags{margin-top:10px;display:flex;gap:6px;flex-wrap:wrap}
.tag{font-size:11px;font-weight:600;padding:2px 8px;border-radius:999px;background:#f1f4f8;color:#475467}
.tag.i{background:#e8f0fd;color:#1d4fb8}.tag.m{background:#e6f7f4;color:#0f766e}
.empty{max-width:640px;padding:28px;background:#fff;border:1px dashed #cfd6e0;border-radius:12px;font-size:14px;line-height:1.6;color:#475467}
.empty code{display:block;margin-top:8px;padding:10px 12px;background:#0f172a;color:#e2e8f0;border-radius:8px;font:12.5px/1.5 "JetBrains Mono",ui-monospace,Menlo,monospace;white-space:pre-wrap}
.seg{display:inline-flex;padding:2px;border-radius:8px;background:rgba(255,255,255,.08)}
.seg a{height:28px;display:inline-flex;align-items:center;padding:0 12px;border-radius:6px;font-size:12.5px;color:#94a3b8}
.seg a.on{background:#fff;color:#0f172a;font-weight:600}
.btn{height:32px;display:inline-flex;align-items:center;gap:6px;padding:0 12px;border-radius:7px;border:1px solid #334155;background:transparent;color:#e2e8f0;font:inherit;font-size:13px;cursor:pointer}
.btn:hover{background:#1e293b}.btn.danger:hover{background:#7f1d1d;border-color:#991b1b}
.frame{width:100%;height:calc(100vh - 52px);border:0;display:block;background:#eef1f5}
.phone-stage{height:calc(100vh - 52px);display:flex;align-items:center;justify-content:center;background:radial-gradient(circle at 50% 30%,#e9eef6,#d9e0ea);overflow:auto;padding:24px}
.phone{position:relative;width:404px;height:min(840px,calc(100vh - 100px));padding:14px;border-radius:54px;background:#111827;box-shadow:0 0 0 2px #374151,0 30px 80px rgba(15,23,42,.35)}
.phone:before{content:"";position:absolute;top:22px;left:50%;width:110px;height:28px;margin-left:-55px;border-radius:16px;background:#000;z-index:2}
.phone iframe{width:100%;height:100%;border:0;border-radius:40px;background:#fff;display:block}
"#;

fn page(ctx: &Ctx, title: &str, top: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · {}</title><style>{CSS}</style></head><body><header class=\"top\"><a class=\"logo\" href=\"/\">{LOGO}</a><a class=\"brand\" href=\"/\">{}</a>{top}</header>{body}</body></html>",
        esc(title),
        esc(&ctx.cfg.name),
        esc(&ctx.cfg.name)
    )
}

/// "3 minutes ago" from an RFC 3339 time (in JS so it's in the viewer's locale).
fn when(m: &Meta) -> String {
    format!("<time datetime=\"{0}\">{0}</time>", esc(&m.published))
}

const TIME_JS: &str = r#"<script>
(function(){var rtf=window.Intl&&Intl.RelativeTimeFormat?new Intl.RelativeTimeFormat(undefined,{numeric:'auto'}):null;
document.querySelectorAll('time').forEach(function(t){var d=new Date(t.getAttribute('datetime'));if(isNaN(d))return;var s=(d-Date.now())/1000,a=Math.abs(s);
var txt=!rtf?d.toLocaleString():a<60?rtf.format(Math.round(s),'second'):a<3600?rtf.format(Math.round(s/60),'minute'):a<86400?rtf.format(Math.round(s/3600),'hour'):a<2592000?rtf.format(Math.round(s/86400),'day'):d.toLocaleDateString();
t.textContent='Published '+txt;t.title=d.toLocaleString();});
var q=document.getElementById('q');if(q)q.addEventListener('input',function(){var v=q.value.toLowerCase();document.querySelectorAll('.card').forEach(function(c){c.style.display=c.textContent.toLowerCase().indexOf(v)<0?'none':'';});});})();
</script>"#;

pub fn home(ctx: &Ctx, ws: Option<&str>, _q: &[(String, String)]) -> String {
    let all = ctx.store.list();
    let wss = ctx.store.workspaces();
    let reports: Vec<&Meta> = all.iter().filter(|m| ws.is_none_or(|w| m.workspace_slug == w)).collect();
    let mut side = format!("<a href=\"/\" class=\"{}\">Home<span>{}</span></a><h3>Workspaces</h3>", if ws.is_none() { "on" } else { "" }, all.len());
    for (name, slug, n) in &wss {
        side += &format!("<a href=\"/w/{slug}\" class=\"{}\">{}<span>{n}</span></a>", if ws == Some(slug.as_str()) { "on" } else { "" }, esc(name));
    }
    if wss.is_empty() {
        side += "<a>None yet</a>";
    }
    let title = ws.and_then(|w| wss.iter().find(|x| x.1 == w)).map(|x| x.0.clone()).unwrap_or_else(|| "All reports".into());
    let mut main = format!("<h1>{}</h1><p class=\"lead\">{} report{}{}</p>", esc(&title), reports.len(), if reports.len() == 1 { "" } else { "s" }, if ws.is_none() && !wss.is_empty() { format!(" in {} workspace{}", wss.len(), if wss.len() == 1 { "" } else { "s" }) } else { String::new() });
    if reports.is_empty() {
        main += "<div class=\"empty\"><b>No reports yet.</b> Publish one from VizCraft (Publish → VizCraft Service), or from the command line:<code>vizcraft-server publish report.vizc --to http://this-server:8080 --workspace \"Sales\" --token $VIZCRAFT_TOKEN</code></div>";
    } else {
        main += "<div class=\"grid\">";
        for m in reports {
            let mut tags = String::new();
            if m.interactive {
                tags += "<span class=\"tag i\">Interactive</span>";
            }
            if m.mobile {
                tags += "<span class=\"tag m\">Mobile layout</span>";
            }
            tags += &format!("<span class=\"tag\">{} page{}</span>", m.pages.len(), if m.pages.len() == 1 { "" } else { "s" });
            main += &format!(
                "<a class=\"card\" href=\"{url}\"><div class=\"thumb\"><img loading=\"lazy\" src=\"/thumb/{ws}/{slug}.svg\" alt=\"\"></div><div class=\"meta\"><div class=\"t\">{name}</div><div class=\"s\">{wsname} · {when}</div><div class=\"tags\">{tags}</div></div></a>",
                url = esc(&m.url),
                ws = m.workspace_slug,
                slug = m.slug,
                name = esc(&m.name),
                wsname = esc(&m.workspace),
                when = when(m)
            );
        }
        main += "</div>";
    }
    let top = "<span class=\"sp\"></span><input id=\"q\" class=\"search\" type=\"search\" placeholder=\"Search reports\" aria-label=\"Search reports\">";
    page(ctx, &title, top, &format!("<div class=\"wrap\"><nav class=\"side\">{side}</nav><main class=\"main\">{main}</main></div>{TIME_JS}"))
}

pub fn report(ctx: &Ctx, m: &Meta, phone: bool) -> String {
    let base = format!("/r/{}/{}", m.workspace_slug, m.slug);
    let seg = if m.mobile {
        format!("<div class=\"seg\"><a href=\"{base}\" class=\"{}\">Desktop</a><a href=\"{base}?view=phone\" class=\"{}\">Phone frame</a></div>", if phone { "" } else { "on" }, if phone { "on" } else { "" })
    } else {
        String::new()
    };
    let top = format!(
        "<div class=\"crumbs\"><span>/</span><a href=\"/w/{}\">{}</a><span>/</span><b>{}</b></div><span class=\"sp\"></span>{seg}<a class=\"btn\" href=\"{base}/files/\" target=\"_blank\" rel=\"noopener\">Open full screen</a><button class=\"btn danger\" id=\"del\">Delete</button>",
        m.workspace_slug,
        esc(&m.workspace),
        esc(&m.name)
    );
    let body = if phone {
        format!("<div class=\"phone-stage\"><div class=\"phone\"><iframe title=\"{}\" src=\"{base}/files/?layout=mobile&amp;embed=1\"></iframe></div></div>", esc(&m.name))
    } else {
        format!("<iframe class=\"frame\" title=\"{}\" src=\"{base}/files/\" allowfullscreen></iframe>", esc(&m.name))
    };
    let script = format!(
        "<script>document.getElementById('del').addEventListener('click',function(){{if(!confirm('Delete {}? This removes it for everyone.'))return;var t=sessionStorage.getItem('vc-token')||prompt('Service token (leave empty if the service has none):')||'';fetch('/api/reports/{}/{}',{{method:'DELETE',headers:t?{{Authorization:'Bearer '+t}}:{{}}}}).then(function(r){{if(r.ok){{sessionStorage.setItem('vc-token',t);location.href='/w/{}';}}else r.json().then(function(j){{sessionStorage.removeItem('vc-token');alert(j.error||'Could not delete.');}});}});}});</script>",
        esc(&m.name).replace('\'', "\\'"),
        m.workspace_slug,
        m.slug,
        m.workspace_slug
    );
    page(ctx, &m.name, &top, &format!("{body}{script}"))
}

pub fn not_found(ctx: &Ctx) -> String {
    page(ctx, "Not found", "", "<main class=\"main\"><div class=\"empty\"><b>There's nothing here.</b> The report may have been deleted or renamed. <a href=\"/\" style=\"color:#1d4fb8\">Back to all reports</a></div></main>")
}
