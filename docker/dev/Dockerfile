FROM ubuntu:24.04

# Install dependencies
RUN apt-get update && apt-get install -y \
    curl \
    git \
    build-essential \
    pkg-config \
    libssl-dev \
    zsh \
    ca-certificates \
    postgresql-client \
    make \
    sudo \
    && rm -rf /var/lib/apt/lists/*

# Install GitHub CLI
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg && \
    chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null && \
    apt-get update && \
    apt-get install -y gh && \
    rm -rf /var/lib/apt/lists/*

# Set zsh as default shell
SHELL ["/bin/zsh", "-c"]

# Create non-root user
RUN useradd -m -s /bin/zsh dev && \
    echo "dev ALL=(ALL) NOPASSWD: ALL" >> /etc/sudoers

USER dev

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/home/dev/.cargo/bin:${PATH}"

# Install cargo-binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Install Volta
RUN curl https://get.volta.sh | bash
ENV VOLTA_HOME="/home/dev/.volta"
ENV PATH="$VOLTA_HOME/bin:$PATH"

# Install Node.js via Volta
RUN volta install node

# Install Rust dev tools via cargo-binstall
RUN cargo binstall -y \
    eza \
    starship \
    ripgrep \
    fd-find \
    bat \
    cargo-nextest \
    cargo-machete \
    cargo-autoinherit \
    sqlx-cli

# Install Claude Code
RUN npm install -g @anthropic-ai/claude-code

# Set up zsh aliases and history for dev user
RUN echo 'alias ls="eza"' >> /home/dev/.zshrc && \
    echo 'alias ll="eza -l --git -b"' >> /home/dev/.zshrc && \
    echo 'alias lt="eza -Tlm --time-style long-iso"' >> /home/dev/.zshrc && \
    echo 'alias lta="eza -Tlma --time-style long-iso"' >> /home/dev/.zshrc && \
    echo 'alias cb="cargo build"' >> /home/dev/.zshrc && \
    echo 'alias ct="cargo test"' >> /home/dev/.zshrc && \
    echo 'alias ntr="cargo nextest run"' >> /home/dev/.zshrc && \
    echo 'alias ntrl="RUST_LOG=info ntr --no-capture"' >> /home/dev/.zshrc && \
    echo 'alias ntrd="RUST_LOG=debug ntr --no-capture"' >> /home/dev/.zshrc && \
    echo 'alias ntrlt="RUST_LOG=trace ntr --no-capture"' >> /home/dev/.zshrc && \
    echo 'alias hurry_install="cargo install --path ./packages/hurry --locked"' >> /home/dev/.zshrc && \
    echo 'alias gc="git clone"' >> /home/dev/.zshrc && \
    echo 'alias ..="cd .."' >> /home/dev/.zshrc && \
    echo 'alias psql="psql -h postgres -U courier -d courier"' >> /home/dev/.zshrc && \
    echo 'alias claude="claude --dangerously-skip-permissions"' >> /home/dev/.zshrc && \
    echo 'alias courier_migrate="cargo sqlx migrate run --source packages/courier/schema/migrations/ --database-url postgres://courier:courier@postgres:5432/courier"' >> /home/dev/.zshrc && \
    echo 'HISTFILE=/home/dev/.zsh_history_data/.zsh_history' >> /home/dev/.zshrc && \
    echo 'HISTSIZE=10000' >> /home/dev/.zshrc && \
    echo 'SAVEHIST=10000' >> /home/dev/.zshrc && \
    echo 'export PGPASSWORD=courier' >> /home/dev/.zshrc && \
    echo 'eval "$(starship init zsh)"' >> /home/dev/.zshrc && \
    mkdir -p /home/dev/.zsh_history_data

# Add entrypoint script
USER root
COPY ./docker/dev/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

USER dev
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["/bin/zsh"]
