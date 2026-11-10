#!/bin/bash
set -e
DEST="/mnt/Disk/downloads/Games/Switch Game/安装包备份"
ssh -i ~/.ssh/id_ed25519 root@192.168.100.150 "mkdir -p '$DEST'"
rsync -a --info=progress2 \
  -e "ssh -i ~/.ssh/id_ed25519" \
  ~/Switch/nsp_share/P5R_ASIA.nsp \
  ~/Switch/nsp_share/P5R_UPDATE.nsp \
  ~/Switch/nsp_share/PokemonArceus_Base.nsp \
  ~/Switch/nsp_share/PokemonArceus_Update.nsp \
  ~/Switch/nsp_share/GTA_VC_Base.nsp \
  ~/Switch/nsp_share/GTA_VC_Update.nsp \
  ~/Switch/nsp_share/0100c2d00ebd4000.nsp \
  ~/Switch/nsp_share/0100c2d00ebd4800.nsp \
  ~/Switch/nsp_share/0100c2d00ebd5001.nsp \
  ~/Switch/nsp_share/0100c2d00ebd5002.nsp \
  ~/Switch/nsp_share/0100c2d00ebd5003.nsp \
  ~/Switch/nsp_share/0100c2d00ebd5004.nsp \
  ~/Switch/nsp_share/0100c2d00ebd5005.nsp \
  "root@192.168.100.150:$DEST/"
ssh -i ~/.ssh/id_ed25519 root@192.168.100.150 "chown -R webdav:webdav '$DEST'; ls -la '$DEST' | wc -l"
echo "NAS_BACKUP_DONE"
