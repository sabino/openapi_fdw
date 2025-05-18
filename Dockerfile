FROM postgres:15

ENV POSTGRES_PASSWORD=postgres
ENV POSTGRES_USER=postgres
ENV POSTGRES_DB=postgres

RUN apt-get update && apt-get install -y python3-pip python3-dev build-essential libpq-dev postgresql-server-dev-all git \
    && pip3 install multicorn hy && rm -rf /var/lib/apt/lists/*

COPY . /src
RUN pip3 install /src

# Use the default command from postgres image

