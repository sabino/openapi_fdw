FROM postgres:15

ENV POSTGRES_PASSWORD=postgres
ENV POSTGRES_USER=postgres
ENV POSTGRES_DB=postgres

RUN apt-get update \
    && apt-get install -y --no-install-recommends curl python3 python3-dev build-essential libpq-dev postgresql-server-dev-all git \
    && rm -rf /var/lib/apt/lists/*

RUN curl -LsSf https://astral.sh/uv/install.sh | sh && mv /root/.local/bin/uv /usr/local/bin/uv

COPY . /src

RUN uv pip install --system multicorn \
    && uv pip install --system /src

# Use the default command from postgres image
