FROM hub.mirrorify.net/python:3.12-slim

RUN sed -i 's|http://deb.debian.org|http://mirrors.aliyun.com|g' /etc/apt/sources.list.d/debian.sources
RUN sed -i 's|http://security.debian.org|https://mirrors.aliyun.com|g' /etc/apt/sources.list.d/debian.sources

RUN apt-get update && \
    apt-get install -y --no-install-recommends libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

RUN pip config set global.index-url https://mirrors.aliyun.com/pypi/simple/
RUN pip config set install.trusted-host mirrors.aliyun.com

# python环境
RUN python3 -m venv /opt/venv && . /opt/venv/bin/activate && \
    pip install --upgrade --no-cache-dir pip

ENV VIRTUAL_ENV=/opt/venv
ENV PATH="${VIRTUAL_ENV}/bin:${PATH}"

WORKDIR /app

# 复制wheel包
COPY ./target/wheels/router-*.whl .

# 安装包
RUN pip install --no-cache-dir /app/router-*.whl
