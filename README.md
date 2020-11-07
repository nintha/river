# River
RTMP Live Stream Server



## Push

OBS

## Play

```shell
ffplay -fflags nobuffer -analyzeduration 100000 rtmp://localhost:11935/channel/token
```

使用`-fflags nobuffer -analyzeduration 100000`可以有效降低播放的延迟，目前本地测试延迟大概为1秒



## TODO

- [ ] 支持不同分辨率的推流和拉流（目前固定1028x720）