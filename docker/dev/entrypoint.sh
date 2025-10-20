#!/bin/bash
set -e

# Fix ownership of Docker volumes (not host mounts)
sudo chown -R dev:dev /workspace/target 2>/dev/null || true
sudo chown -R dev:dev /home/dev/.zsh_history_data 2>/dev/null || true
sudo chown -R dev:dev /home/dev/.cargo/registry 2>/dev/null || true
sudo chown -R dev:dev /home/dev/.cargo/git 2>/dev/null || true
sudo chown -R dev:dev /home/dev/.ssh 2>/dev/null || true

# Fix SSH permissions (SSH requires strict permissions)
if [ -d /home/dev/.ssh ]; then
    sudo chmod 700 /home/dev/.ssh
    sudo find /home/dev/.ssh -type f -name "id_*" ! -name "*.pub" -exec chmod 600 {} \;
fi

# Execute the command
exec "$@"
