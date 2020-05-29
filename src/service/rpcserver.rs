use super::{Instruction, TxType};
use crate::{config, myrpc};
use grpcio;
use log::{error, info};
use std::fs::File;
use std::io::{Error, ErrorKind, Read, Result};
use std::sync::Arc;

pub struct RpcServer {
    server: Option<grpcio::Server>,
    addr: String,
    port: u16,
    pkcs12: String,
    pkcs12_passwd: String,
}

impl RpcServer {
    pub fn new(cfg: &config::ServerCfg) -> RpcServer {
        RpcServer {
            server: None,
            addr: cfg.my_grpc_addr.to_string(),
            port: cfg.my_grpc_port,
            pkcs12: cfg.pkcs12.to_string(),
            pkcs12_passwd: cfg.pkcs12_password.to_string(),
        }
    }

    pub fn start(&mut self, s: TxType) -> Result<()> {
        let e = Arc::new(grpcio::Environment::new(1));
        let sb = grpcio::ServerBuilder::new(e);
        let credentials = pkcs12_to_server_credentials(&self.pkcs12, &self.pkcs12_passwd)?;
        let sb = sb.bind_with_cred(&self.addr, self.port, credentials);
        let dvi = DvExportImpl { ref_service_tx: s };
        let service = myrpc::create_dv_export(dvi);
        let sb = sb.register_service(service);

        let mut server = sb.build().map_err(|e| Error::new(ErrorKind::Other, e))?;
        server.start();

        info!(
            "[gRpcServer] startup ok, listen at {}:{}",
            self.addr, self.port
        );
        self.server = Some(server);

        Ok(())
    }
}

#[derive(Clone)]
struct DvExportImpl {
    ref_service_tx: TxType,
}

impl myrpc::DvExport for DvExportImpl {
    fn kickout_uuid(
        &mut self,
        _ctx: ::grpcio::RpcContext,
        req: myrpc::Kickout,
        sink: ::grpcio::UnarySink<myrpc::Result>,
    ) {
        //
        info!("[gRpcServer] kickout_uuid called, uuid:{}", req.uuid);
        let ins = Instruction::Kickout(req.uuid, sink);

        match self.ref_service_tx.send(ins) {
            Err(e) => {
                error!("[gRpcServer] kickout_uuid unbounded_send failed:{}", e);
            }
            _ => {}
        }
    }

    fn uuid_cfg_changed(
        &mut self,
        _ctx: ::grpcio::RpcContext,
        req: myrpc::CfgChangeNotify,
        sink: ::grpcio::UnarySink<myrpc::Result>,
    ) {
        //
        info!("[gRpcServer] uuid_cfg_changed called, uuid:{}", req.uuid);
        let ins = Instruction::CfgChangeNotify(req, sink);

        match self.ref_service_tx.send(ins) {
            Err(e) => {
                error!("[gRpcServer] uuid_cfg_changed unbounded_send failed:{}", e);
            }
            _ => {}
        }
    }
}

fn pkcs12_to_server_credentials(filepath: &str, passw: &str) -> Result<grpcio::ServerCredentials> {
    let mut file = File::open(filepath)?;
    let mut identity = vec![];
    file.read_to_end(&mut identity)?;
    let pkcs12 = openssl::pkcs12::Pkcs12::from_der(&identity)?;
    let parsed = pkcs12.parse(passw)?;

    let sb = grpcio::ServerCredentialsBuilder::new();
    let cert = parsed.cert.to_pem()?;
    let private_key = parsed.pkey.private_key_to_pem_pkcs8()?;
    let sb = sb.add_cert(cert, private_key);

    Ok(sb.build())
}
