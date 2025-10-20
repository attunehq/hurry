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
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Install cargo-binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Install Volta
RUN curl https://get.volta.sh | bash
ENV VOLTA_HOME="/root/.volta"
ENV PATH="$VOLTA_HOME/bin:$PATH"

# Install Node.js via Volta
RUN volta install node

# Install eza via cargo-binstall
RUN cargo binstall -y eza

# Install Claude Code
RUN npm install -g @anthropic-ai/claude-code

# Set zsh as default shell
SHELL ["/bin/zsh", "-c"]
RUN chsh -s /bin/zsh

# Set up zsh aliases and history
RUN echo 'alias ls="eza"' >> /root/.zshrc && \
    echo 'alias ll="eza -l --git -b"' >> /root/.zshrc && \
    echo 'alias lt="eza -Tlm --time-style long-iso"' >> /root/.zshrc && \
    echo 'alias lta="eza -Tlma --time-style long-iso"' >> /root/.zshrc && \
    echo 'alias ntr="cargo nextest run"' >> /root/.zshrc && \
    echo 'alias ntrl="RUST_LOG=info ntr --no-capture"' >> /root/.zshrc && \
    echo 'alias ntrd="RUST_LOG=debug ntr --no-capture"' >> /root/.zshrc && \
    echo 'alias ntrlt="RUST_LOG=trace ntr --no-capture"' >> /root/.zshrc && \
    echo 'alias gc="git clone"' >> /root/.zshrc && \
    echo 'HISTFILE=/root/.zsh_history_data/.zsh_history' >> /root/.zshrc && \
    echo 'HISTSIZE=10000' >> /root/.zshrc && \
    echo 'SAVEHIST=10000' >> /root/.zshrc && \
    mkdir -p /root/.zsh_history_data

WORKDIR /workspace

CMD ["/bin/zsh"]
