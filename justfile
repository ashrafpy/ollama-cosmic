name := 'cosmic-applet-ollama'
appid := 'dev.ashraf.CosmicAppletOllama'

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
appdata-dst := base-dir / 'share' / 'metainfo' / appid + '.metainfo.xml'
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '-symbolic.svg'
icon-active-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '-active-symbolic.svg'

default: build-release

clean:
    cargo clean

build-debug *args:
    cargo build {{args}}

build-release *args: (build-debug '--release' args)

check *args:
    cargo check --all-targets {{args}}

test *args:
    cargo test {{args}}

install:
    install -Dm0755 target/release/{{name}} {{bin-dst}}
    install -Dm0644 res/{{appid}}.desktop {{desktop-dst}}
    install -Dm0644 res/{{appid}}.metainfo.xml {{appdata-dst}}
    install -Dm0644 res/icons/{{appid}}-symbolic.svg {{icon-dst}}
    install -Dm0644 res/icons/{{appid}}-active-symbolic.svg {{icon-active-dst}}

uninstall:
    rm -f {{bin-dst}} {{desktop-dst}} {{appdata-dst}} {{icon-dst}} {{icon-active-dst}}
