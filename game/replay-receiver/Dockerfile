FROM python:3.13-alpine AS test
WORKDIR /src
COPY lumagame_replay.py test_lumagame_replay.py ./
RUN python3 -m unittest test_lumagame_replay.py

FROM python:3.13-alpine
RUN apk add --no-cache tzdata
COPY --from=test /src/lumagame_replay.py /app/lumagame_replay.py
ENV PORT=8765 BUFFER_DIR=/buffer RECORDINGS_DIR=/recordings PYTHONUNBUFFERED=1
EXPOSE 8765
# Unraid's nobody:users, the owner of the share the recordings go to.
USER 99:100
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD python3 -c "import os,urllib.request;urllib.request.urlopen('http://127.0.0.1:'+os.environ.get('PORT','8765')+'/health',timeout=4)"
ENTRYPOINT ["python3", "/app/lumagame_replay.py"]
