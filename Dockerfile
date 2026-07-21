FROM python:3.12-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY pyproject.toml ./
COPY src/ ./src/
RUN pip install --no-cache-dir .
ENV PYTHONUNBUFFERED=1
CMD ["turbomcp", "serve", "src/liberado_python_interpreter/server.py"]
