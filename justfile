install-dev:
    sudo mkdir -p /opt/gah/bin
    cargo build
    sudo systemctl stop gah-api gah-webui || true
    sudo cp target/debug/gah /opt/gah/bin/gah
